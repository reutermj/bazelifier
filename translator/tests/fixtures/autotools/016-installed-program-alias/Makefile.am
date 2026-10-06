bin_PROGRAMS = greeter
greeter_SOURCES = main.c

# The second name, exactly as the corpus projects spell it.
install-exec-hook:
	(cd $(DESTDIR)$(bindir); rm -f farewell$(EXEEXT); $(LN_S) greeter$(EXEEXT) farewell$(EXEEXT))
