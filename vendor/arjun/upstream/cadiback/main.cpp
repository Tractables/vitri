#include "include/cadiback.h"
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

static void usage(const char* name) {
  fprintf(stderr, "usage: %s [--verbose] <input.cnf>\n", name);
  exit(1);
}

static bool parse_cnf(const char* path, std::vector<int>& cnf) {
  FILE* f = fopen(path, "r");
  if (!f) {
    fprintf(stderr, "error: cannot open '%s'\n", path);
    return false;
  }
  int ch;
  while ((ch = fgetc(f)) != EOF) {
    if (ch == 'c') {
      while ((ch = fgetc(f)) != EOF && ch != '\n')
        ;
      continue;
    }
    if (ch == 'p') {
      while ((ch = fgetc(f)) != EOF && ch != '\n')
        ;
      continue;
    }
    if (ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r')
      continue;
    if (ch == '-' || (ch >= '0' && ch <= '9')) {
      ungetc(ch, f);
      int lit;
      if (fscanf(f, "%d", &lit) != 1) {
        fprintf(stderr, "error: parse error in '%s'\n", path);
        fclose(f);
        return false;
      }
      cnf.push_back(lit);
    } else {
      fprintf(stderr, "error: unexpected character '%c' in '%s'\n", ch, path);
      fclose(f);
      return false;
    }
  }
  fclose(f);
  return true;
}

int main(int argc, char** argv) {
  bool verbose = false;
  const char* input = nullptr;

  for (int i = 1; i < argc; i++) {
    if (strcmp(argv[i], "--verbose") == 0 || strcmp(argv[i], "-v") == 0) {
      verbose = true;
    } else if (argv[i][0] == '-') {
      fprintf(stderr, "error: unknown option '%s'\n", argv[i]);
      usage(argv[0]);
    } else {
      if (input) {
        fprintf(stderr, "error: multiple input files\n");
        usage(argv[0]);
      }
      input = argv[i];
    }
  }

  if (!input)
    usage(argv[0]);

  std::vector<int> cnf;
  if (!parse_cnf(input, cnf))
    return 1;

  int verb = verbose ? 1 : -1;
  std::vector<int> drop_cands;
  std::vector<int> backbone;
  std::vector<int> red_cls;
  std::vector<std::pair<int, int>> eqlits;

  int result = CadiBack::doit(cnf, verb, drop_cands, backbone, red_cls, eqlits);

  if (result == 20) {
    printf("s UNSATISFIABLE\n");
    return 20;
  }

  for (int lit : backbone)
    printf("%d 0\n", lit);

  return 10;
}
