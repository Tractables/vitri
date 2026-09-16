/* Prepares a CNF with vitri, writes the bundle into a directory as
 * `vitri -o OUT_DIR` does, and prints the run's summary.
 *
 *   prepare INPUT.cnf OUT_DIR [REQUEST_JSON]
 *
 * For example:
 *
 *   prepare docs/example.cnf out '{"vtree": "minfill-primal"}'
 *
 * Building and linking this program is covered in bindings/c/README.md. */

#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#include "vitri.h"

/* The whole file at `path`, in a buffer the caller frees, or NULL. */
static unsigned char *read_file(const char *path, size_t *size) {
  FILE *file = fopen(path, "rb");
  unsigned char *data = NULL;
  size_t capacity = 0;

  *size = 0;
  if (file == NULL) return NULL;
  for (;;) {
    if (*size == capacity) {
      unsigned char *grown;
      capacity = capacity ? 2 * capacity : 65536;
      grown = realloc(data, capacity);
      if (grown == NULL) break;
      data = grown;
    }
    size_t n = fread(data + *size, 1, capacity - *size, file);
    *size += n;
    if (n == 0) break;
  }
  if (ferror(file) || *size == capacity) {
    free(data);
    data = NULL;
  }
  fclose(file);
  return data;
}

/* Create each directory above `path`. */
static int make_parents(char *path) {
  for (char *slash = strchr(path + 1, '/'); slash; slash = strchr(slash + 1, '/')) {
    int made;
    *slash = '\0';
    made = mkdir(path, 0777) == 0 || errno == EEXIST;
    *slash = '/';
    if (!made) return 0;
  }
  return 1;
}

/* Write file `index` of the bundle under `out_dir`. */
static int write_bundle_file(const vitri_result *result, size_t index, const char *out_dir) {
  size_t path_len, contents_len;
  const char *path = vitri_result_file_path(result, index, &path_len);
  const uint8_t *contents = vitri_result_file_contents(result, index, &contents_len);
  size_t size = strlen(out_dir) + 1 + path_len + 1;
  char *target = malloc(size);
  FILE *file = NULL;
  int ok = 0;

  if (target == NULL) return 0;
  snprintf(target, size, "%s/%.*s", out_dir, (int)path_len, path);
  if (make_parents(target) && (file = fopen(target, "wb")) != NULL) {
    ok = fwrite(contents, 1, contents_len, file) == contents_len;
    ok = fclose(file) == 0 && ok;
  }
  if (!ok) perror(target);
  free(target);
  return ok;
}

int main(int argc, char **argv) {
  const char *request = argc == 4 ? argv[3] : NULL;
  unsigned char *dimacs;
  size_t dimacs_len, summary_len;
  vitri_result *result = NULL;
  vitri_code code;
  const char *summary;
  int status = 0;

  if (argc != 3 && argc != 4) {
    fprintf(stderr, "usage: %s INPUT.cnf OUT_DIR [REQUEST_JSON]\n", argv[0]);
    return 2;
  }
  if (vitri_abi_version() != VITRI_ABI_VERSION) {
    fprintf(stderr, "the vitri library has ABI %u, this program was built for %u\n",
            vitri_abi_version(), (unsigned)VITRI_ABI_VERSION);
    return 1;
  }
  dimacs = read_file(argv[1], &dimacs_len);
  if (dimacs == NULL) {
    perror(argv[1]);
    return 1;
  }

  /* Without a request argument, NULL asks for every default. */
  code = vitri_prepare(dimacs, dimacs_len, request, request ? strlen(request) : 0, &result);
  free(dimacs);
  if (code != VITRI_OK) {
    fprintf(stderr, "vitri %s error: %s\n", vitri_result_error_kind(result, NULL),
            vitri_result_error_message(result, NULL));
    vitri_result_free(result);
    return 1;
  }

  for (size_t i = 0; i < vitri_result_file_count(result); i++)
    if (!write_bundle_file(result, i, argv[2])) status = 1;

  summary = vitri_result_summary_json(result, &summary_len);
  fwrite(summary, 1, summary_len, stdout);
  putchar('\n');

  vitri_result_free(result);
  return status;
}
