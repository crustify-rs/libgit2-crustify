/* Compares <ctype.h> in the locale argv[1] names with ctype_compat.h's GIT_WIN32 branch, for every byte. */
#include <ctype.h>
#include <locale.h>
#include <stdio.h>
int main(int argc, char **argv) {
  if (!setlocale(LC_ALL, argv[1])) { printf("no locale %s\n", argv[1]); return 1; }
  int diff = 0;
  for (int c = 0; c < 256; c++) {
    int a = !!isalpha(c), d = !!isdigit(c), n = !!isalnum(c), p = !!isprint(c), s = !!isspace(c), x = !!isxdigit(c);
    int ea = (c>='A'&&c<='Z')||(c>='a'&&c<='z'), ed = c>='0'&&c<='9', es = c==' '||c=='\t'||c=='\n'||c=='\f'||c=='\r'||c=='\v';
    int ex = ed||(c>='a'&&c<='f')||(c>='A'&&c<='F'), ep = c>=' '&&c<='~';
    int el = (c>='A'&&c<='Z')?c+32:c, eu = (c>='a'&&c<='z')?c-32:c;
    if (a!=ea||d!=ed||n!=(ea||ed)||p!=ep||s!=es||x!=ex||tolower(c)!=el||toupper(c)!=eu) { diff++; printf("%s differs at %d\n", argv[1], c); }
  }
  printf("%s: %d bytes differ from GIT_WIN32's comparisons\n", argv[1], diff);
  return 0;
}
