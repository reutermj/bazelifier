#ifndef GREET_H
#define GREET_H

/* Public header declared via install(FILES ... DESTINATION <include>) but NOT
 * listed as a source of the greet library. greet.c and main.c both include it;
 * the translator must copy it (injected as a public header) or neither
 * compiles. */
int greet_value(void);

#endif
