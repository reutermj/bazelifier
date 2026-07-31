//! The CTest frontend: recovers a project's registered tests (`add_test`)
//! and decides which of them the translator can actually express.
//!
//! Separate from `cmake_api` because it reads a **different source**. The
//! File API has no test model at all, so none of this can come from the
//! codemodel reply — it comes from `ctest --show-only=json-v1`, run after
//! the build so test binaries' paths resolve. See
//! docs/lore/cmake-test-model-lives-in-ctest-not-file-api.md.
//!
//! The pipeline is three ordered steps, and `cmake_api::discover` runs them
//! in this order:
//!
//! 1. [`read_tests`] — shell out to ctest and parse its reply.
//! 2. [`rebase_tests_to_module_root`] — absolute paths CTest reported become
//!    module-relative, matching what target paths get.
//! 3. [`partition_tests_by_buildable_command`] — split off the tests whose
//!    command isn't something this module builds, which escalate.
//!
//! Step 3 must follow step 2: the escalation quotes the command verbatim for
//! an agent reading it in the unpacked workspace, where an unrebased path
//! from this machine names nothing.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::model::{Target, TargetKind, Test};
use crate::needs_attention::{NeedsAttention, ctest_command_not_a_target_needs_attention};
use crate::paths::normalize_lexically;

use crate::error::Error;

/// `ctest --show-only=json-v1` output. The File API has no test model, so
/// registered tests (add_test) and their properties come from here instead
/// — see docs/lore/cmake-test-model-lives-in-ctest-not-file-api.md.
#[derive(Debug, Deserialize)]
pub(crate) struct CtestReply {
    tests: Vec<CtestTest>,
}

#[derive(Debug, Deserialize)]
struct CtestTest {
    name: String,
    // The resolved command line: [executable, args...]. Absent until the
    // test's executable is actually built (ctest can't resolve the path
    // before then); the translator builds first, so it is present.
    #[serde(default)]
    command: Vec<String>,
    #[serde(default)]
    properties: Vec<CtestProperty>,
}

#[derive(Debug, Deserialize)]
struct CtestProperty {
    name: String,
    // Property values are polymorphic in the ctest schema (a string for
    // WORKING_DIRECTORY, a list for PASS_REGULAR_EXPRESSION); kept as raw
    // JSON and interpreted per-property by `read_tests`.
    value: serde_json::Value,
}

/// Runs `ctest --show-only=json-v1` in the build directory and translates
/// each registered test into a `Test`. The File API has no test
/// model, so this is the only structured source for `add_test` and its
/// properties — see docs/lore/cmake-test-model-lives-in-ctest-not-file-api.md.
///
/// Working directories are returned ABSOLUTE (as CTest reports them);
/// `rebase_tests_to_module_root` makes them module-relative once the module
/// root is known, matching how target source paths are handled.
///
/// A ctest invocation that fails (no CTest, no tests configured) yields an
/// empty test list rather than an error: a project with no registered tests
/// is the common case, not a failure. A test CTest reports with an empty
/// command is skipped — there is nothing to wrap. That is distinct from a
/// test whose command resolved to something this module doesn't build,
/// which is escalated: see `partition_tests_by_buildable_command`.
pub(crate) fn read_tests(build_dir: &Path) -> Result<Vec<Test>, Error> {
    let output = Command::new("ctest")
        .arg("--show-only=json-v1")
        .current_dir(build_dir)
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let reply: CtestReply = serde_json::from_slice(&output.stdout)?;
    Ok(ctest_reply_to_tests(reply))
}

/// Translates a parsed `ctest --show-only=json-v1` reply into the model's
/// tests. Split from `read_tests` so a frozen real capture can drive it
/// without shelling out. Working directories are still absolute here — see
/// `rebase_tests_to_module_root`.
pub(crate) fn ctest_reply_to_tests(reply: CtestReply) -> Vec<Test> {
    let mut tests = Vec::new();
    for test in reply.tests {
        // The first command element is the executable; its basename is the
        // generated cc_binary this test runs. No command means CTest could
        // not resolve the test's binary, so there is nothing to wrap.
        let Some(executable) = test.command.first() else {
            continue;
        };
        let target = Path::new(executable)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| executable.clone());

        let mut working_directory = String::new();
        let mut pass_regex = None;
        for property in &test.properties {
            match property.name.as_str() {
                "WORKING_DIRECTORY" => {
                    if let Some(dir) = property.value.as_str() {
                        working_directory = dir.to_string();
                    }
                }
                // CMake stores this as a list; a test rarely declares more
                // than one, and the tinyxml2-shaped scope takes the first.
                "PASS_REGULAR_EXPRESSION" => {
                    pass_regex = property
                        .value
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                }
                _ => {}
            }
        }

        tests.push(Test {
            name: test.name,
            target,
            command: executable.clone(),
            working_directory,
            pass_regex,
        });
    }
    tests
}

/// Splits tests into those the translator can emit an `sh_test` for and one
/// escalation covering the rest.
///
/// A generated `sh_test` wraps a `cc_binary` this module emits, so a test
/// whose command is anything else has nothing to point at. The check is
/// against the emitted targets, NOT against the command's path or extension:
/// json-c's commands are checked-in `.test` shell scripts under the source
/// tree while tinyxml2's is a built executable under the build tree, and no
/// rule about where a command lives distinguishes "a binary we built" from
/// "a file that happens to sit there" — only the target list does.
///
/// Emitting a broken label instead would fail at analysis time in the
/// unpacked workspace with `missing input file`, which says nothing about
/// the construct that wasn't understood.
pub(crate) fn partition_tests_by_buildable_command(
    tests: Vec<Test>,
    targets: &[Target],
) -> (Vec<Test>, Option<NeedsAttention>) {
    let executables: HashSet<&str> = targets
        .iter()
        .filter(|t| t.kind == TargetKind::Executable)
        .map(|t| t.name.as_str())
        .collect();

    let (buildable, unbuildable): (Vec<Test>, Vec<Test>) = tests
        .into_iter()
        .partition(|test| executables.contains(test.target.as_str()));

    if unbuildable.is_empty() {
        return (buildable, None);
    }

    let names: Vec<String> = unbuildable.iter().map(|t| t.name.clone()).collect();
    let commands: Vec<String> = unbuildable.iter().map(|t| t.command.clone()).collect();
    (
        buildable,
        Some(ctest_command_not_a_target_needs_attention(&names, &commands)),
    )
}

/// Rewrites each test's `working_directory` from the absolute path CTest
/// reported to one relative to `module_root`, in place — the same rebasing
/// target paths get, but for tests. A working directory at the module root
/// becomes empty. One that resolves outside the module root is left as the
/// empty string (run at the module root): the tinyxml2-shaped scope does not
/// yet escalate a test whose data lives outside the deliverable, and running
/// at the root is the safe default rather than baking in an absolute path.
pub(crate) fn rebase_tests_to_module_root(tests: &mut [Test], module_root: &Path) {
    for test in tests {
        let absolute = normalize_lexically(Path::new(&test.working_directory));
        test.working_directory = absolute
            .strip_prefix(module_root)
            .map(|rel| rel.to_string_lossy().into_owned())
            .unwrap_or_default();

        // The command is rebased too, because it is quoted verbatim into an
        // escalation an agent reads in the unpacked workspace, where this
        // machine's absolute path (a Bazel sandbox path, under a hash that
        // changes every run) names nothing. Unlike working_directory, a
        // command OUTSIDE the module root is kept absolute rather than
        // emptied: it still identifies which command was meant, and losing
        // it would leave the escalation naming no command at all.
        let absolute = normalize_lexically(Path::new(&test.command));
        if let Ok(relative) = absolute.strip_prefix(module_root) {
            test.command = relative.to_string_lossy().into_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TargetKind;

    // Real `ctest --show-only=json-v1` output for tinyxml2's xmltest,
    // captured verbatim. Pins the parse of the test model — command,
    // WORKING_DIRECTORY, PASS_REGULAR_EXPRESSION — against what CTest
    // actually emits, since the File API has no test model to cross-check
    // against. See docs/lore/cmake-test-model-lives-in-ctest-not-file-api.md.
    const CTEST_XMLTEST_JSON: &str = r#"{
  "kind": "ctestInfo",
  "version": { "major": 1, "minor": 0 },
  "tests": [
    {
      "name": "xmltest",
      "command": [ "/abs/build/xmltest" ],
      "properties": [
        { "name": "PASS_REGULAR_EXPRESSION", "value": [", Fail 0"] },
        { "name": "WORKING_DIRECTORY", "value": "/abs/src" }
      ]
    }
  ]
}"#;

    #[test]
    fn ctest_reply_parses_real_capture_into_a_test() {
        let reply: CtestReply =
            serde_json::from_str(CTEST_XMLTEST_JSON).expect("real ctest capture should parse");
        let tests = ctest_reply_to_tests(reply);
        assert_eq!(tests.len(), 1);
        let t = &tests[0];
        assert_eq!(t.name, "xmltest");
        assert_eq!(
            t.target, "xmltest",
            "the test target is the basename of the command's executable"
        );
        assert_eq!(
            t.working_directory, "/abs/src",
            "WORKING_DIRECTORY (a string property) must be read verbatim"
        );
        assert_eq!(
            t.pass_regex.as_deref(),
            Some(", Fail 0"),
            "PASS_REGULAR_EXPRESSION (a list property) must yield its first entry"
        );
    }

    // A test CTest reports with an EMPTY command has no binary to wrap and is
    // skipped, rather than emitting a test that can't run. A command that
    // resolved but names something we don't build is a different case, and is
    // escalated — see the partition_tests_by_buildable_command tests.
    #[test]
    fn ctest_reply_skips_a_test_with_no_command() {
        let reply = CtestReply {
            tests: vec![CtestTest {
                name: "unbuilt".to_string(),
                command: vec![],
                properties: vec![],
            }],
        };
        assert!(ctest_reply_to_tests(reply).is_empty());
    }

    // Real `ctest --show-only=json-v1` output for json-c 0.19's `test1`,
    // captured from a real configure. The mirror image of CTEST_XMLTEST_JSON
    // above, and the pair is the point: json-c's command is a checked-in
    // shell script in the SOURCE tree while its WORKING_DIRECTORY is in the
    // BUILD tree, and tinyxml2's is the other way round. See bzl-fxa.16.
    const CTEST_JSON_C_SCRIPT_JSON: &str = r#"{
  "kind": "ctestInfo",
  "version": { "major": 1, "minor": 0 },
  "tests": [
    {
      "backtrace": 3,
      "command": [ "/abs/src/tests/test1.test" ],
      "name": "test1",
      "properties": [
        { "name": "WORKING_DIRECTORY", "value": "/abs/build/tests" }
      ]
    }
  ]
}"#;

    #[test]
    fn ctest_reply_carries_the_command_path_not_just_its_basename() {
        let reply: CtestReply = serde_json::from_str(CTEST_JSON_C_SCRIPT_JSON)
            .expect("real json-c ctest capture should parse");
        let tests = ctest_reply_to_tests(reply);
        assert_eq!(tests.len(), 1);
        assert_eq!(
            tests[0].command, "/abs/src/tests/test1.test",
            "the full command path must survive: the basename alone cannot tell a \
             checked-in script from a binary the module builds"
        );
    }

    fn executable_target(name: &str) -> Target {
        Target {
            name: name.to_string(),
            kind: TargetKind::Executable,
            sources: Vec::new(),
            public_headers: Vec::new(),
            dependencies: Vec::new(),
            includes: Vec::new(),
            local_defines: Vec::new(),
            artifacts: Vec::new(),
            ..Default::default()
        }
    }

    fn test_running(name: &str, target: &str, command: &str) -> Test {
        Test {
            name: name.to_string(),
            target: target.to_string(),
            command: command.to_string(),
            working_directory: String::new(),
            pass_regex: None,
        }
    }

    // The direction that must NOT escalate: tinyxml2's shape, where the
    // command is an executable the module emits. Without this, an escalation
    // wired to fire on everything would look correct.
    #[test]
    fn a_test_running_a_built_binary_does_not_escalate() {
        let tests = vec![test_running("xmltest", "xmltest", "/abs/build/xmltest")];
        let targets = vec![executable_target("xmltest")];

        let (kept, escalation) = partition_tests_by_buildable_command(tests, &targets);

        assert_eq!(kept.len(), 1, "the test must still be emitted");
        assert!(
            escalation.is_none(),
            "a test wrapping a cc_binary this module emits is fully translated, so \
             nothing should escalate; got: {:?}",
            escalation.map(|e| e.title)
        );
    }

    // The direction that must escalate: json-c's shape.
    #[test]
    fn a_test_running_a_script_escalates_and_is_not_emitted() {
        let tests = vec![test_running("test1", "test1.test", "/abs/src/tests/test1.test")];
        // The module builds an executable, just not the one the test runs —
        // so an empty target list isn't what makes this fire.
        let targets = vec![executable_target("json_parse")];

        let (kept, escalation) = partition_tests_by_buildable_command(tests, &targets);

        assert!(
            kept.is_empty(),
            "a test whose command isn't a target we emit has no cc_binary to wrap, so \
             emitting an sh_test for it would produce a label that can't resolve"
        );
        let item = escalation.expect("a test that can't be translated must escalate");
        assert!(
            item.gap.contains("/abs/src/tests/test1.test"),
            "the escalation must name the actual command, which the agent can't \
             recover from the test name:\n{}",
            item.gap
        );
        assert!(
            item.gap.contains("test1"),
            "the escalation must name the test:\n{}",
            item.gap
        );
    }

    // A project mixing both shapes must keep the translatable half rather
    // than escalating the whole suite.
    #[test]
    fn a_mixed_suite_keeps_the_buildable_tests_and_escalates_only_the_rest() {
        let tests = vec![
            test_running("xmltest", "xmltest", "/abs/build/xmltest"),
            test_running("test1", "test1.test", "/abs/src/tests/test1.test"),
        ];
        let targets = vec![executable_target("xmltest")];

        let (kept, escalation) = partition_tests_by_buildable_command(tests, &targets);

        assert_eq!(
            kept.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            vec!["xmltest"],
            "only the test whose command is a target we emit survives"
        );
        let item = escalation.expect("the untranslatable test must still escalate");
        assert!(
            !item.gap.contains("xmltest"),
            "the translated test must not appear in the escalation:\n{}",
            item.gap
        );
    }

    #[test]
    fn rebase_tests_makes_working_directory_module_relative() {
        let mut tests = vec![
            Test {
                name: "at_root".to_string(),
                target: "t".to_string(),
                command: "/proj/build/t".to_string(),
                working_directory: "/proj".to_string(),
                pass_regex: None,
            },
            Test {
                name: "in_subdir".to_string(),
                target: "t".to_string(),
                command: "/proj/build/t".to_string(),
                working_directory: "/proj/tests/data".to_string(),
                pass_regex: None,
            },
        ];
        rebase_tests_to_module_root(&mut tests, Path::new("/proj"));
        // The module root itself becomes empty; a subdir becomes relative.
        assert_eq!(tests[0].working_directory, "");
        assert_eq!(tests[1].working_directory, "tests/data");
    }

    #[test]
    fn rebase_tests_makes_the_command_module_relative() {
        let mut tests = vec![
            test_running("inside", "t", "/proj/tests/check.sh"),
            test_running("outside", "t", "/elsewhere/check.sh"),
        ];
        rebase_tests_to_module_root(&mut tests, Path::new("/proj"));

        assert_eq!(
            tests[0].command, "tests/check.sh",
            "a command under the module root must be rebased: the escalation quoting it \
             is read in the unpacked workspace, where this machine's absolute path names \
             nothing"
        );
        assert_eq!(
            tests[1].command, "/elsewhere/check.sh",
            "a command outside the module root stays absolute — unlike working_directory, \
             emptying it would leave the escalation naming no command at all"
        );
    }
}
