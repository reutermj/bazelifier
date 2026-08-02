#include <config.h>

#include <stdio.h>

/* Prints what the config header actually resolved to, so the ground-truth
   comparison fails if a probe answers differently than it did for CMake. */
int main(void)
{
	printf("package=%s version=%s\n", PACKAGE_NAME, PACKAGE_VERSION);
#ifdef HAVE_STDLIB_H
	printf("stdlib=yes\n");
#else
	printf("stdlib=no\n");
#endif
#ifdef HAVE_UNISTD_H
	printf("unistd=yes\n");
#else
	printf("unistd=no\n");
#endif
	return 0;
}
