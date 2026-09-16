/* Tests the C API through its header: bundles equal byte for byte to what the
 * vitri executable writes, runs that end without a vtree, error codes and
 * messages, the pointer and length contracts, repeated and concurrent calls,
 * and where the Arjun stage runs.
 *
 *   test_vitri VITRI_EXECUTABLE SCRATCH_DIR [CNF...]
 *
 * VITRI_EXECUTABLE is the vitri binary built from the same source. Each CNF
 * named, and each formula below, goes through vitri_prepare and through that
 * binary, and the two bundles are compared. */

#define _XOPEN_SOURCE 700

#include <errno.h>
#include <fcntl.h>
#include <ftw.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#include "vitri.h"

static int failures = 0;

#define CHECK(cond, ...)                                                       \
  do {                                                                         \
    if (!(cond)) {                                                             \
      fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__);                     \
      fprintf(stderr, __VA_ARGS__);                                            \
      fputc('\n', stderr);                                                     \
      failures++;                                                              \
    }                                                                          \
  } while (0)

#define PATH_SIZE 4096

static const char *vitri_executable;
static const char *scratch;

/* Two components of three variables each. */
static const char TWO_COMPONENTS[] =
    "p cnf 6 6\n1 2 3 0\n-1 -2 0\n-2 -3 0\n4 5 6 0\n-4 -5 0\n-5 -6 0\n";
/* Preprocessing leaves this one with variables to build a vtree over. */
static const char IRREDUCIBLE[] =
    "p cnf 5 5\n1 2 0\n-1 3 0\n-2 -3 4 0\n2 3 -4 0\n4 5 0\n";
/* Every variable forced or free: nothing is left to build a vtree over. */
static const char FULLY_RESOLVED[] = "p cnf 3 2\n1 0\n2 0\n";
/* Refuted by unit propagation. */
static const char REFUTED[] = "p cnf 2 2\n1 0\n-1 0\n";
static const char MALFORMED[] = "p cnf 2 1\n1 x 0\n";

/* One cheap elimination order, so a bundle does not depend on which
 * candidate a portfolio picked. */
static const char MINFILL[] = "{\"vtree\": \"minfill-primal\"}";
static const char MINFILL_DOT[] = "{\"vtree\": \"minfill-primal\", \"dot\": true}";
/* Preprocessing resolves TWO_COMPONENTS outright; with it off, both
 * components get files of their own. */
static const char UNPREPROCESSED[] =
    "{\"vtree\": \"minfill-primal\", \"dot\": true, \"simplify\": false, \"arjun\": false}";

/* The vitri flags each request above stands for. */
static const char *const MINFILL_FLAGS[] = {NULL};
static const char *const MINFILL_DOT_FLAGS[] = {"--dot", NULL};
static const char *const UNPREPROCESSED_FLAGS[] = {"--dot", "--no-simplify", "--no-arjun", NULL};

/* vitri_prepare over NUL-terminated text; a NULL request is the default. */
static vitri_code prepare(const char *dimacs, const char *request, vitri_result **out) {
  return vitri_prepare((const uint8_t *)dimacs, strlen(dimacs), request,
                       request ? strlen(request) : 0, out);
}

/* Every buffer vitri lends is followed by a NUL, so it prints as a string. */
static const char *or_null(const char *text) { return text ? text : "(null)"; }

static const char *kind_of(const vitri_result *result) {
  return or_null(vitri_result_error_kind(result, NULL));
}

static const char *message_of(const vitri_result *result) {
  return or_null(vitri_result_error_message(result, NULL));
}

/* ------------------------------------------------------------ file helpers */

static void write_text(const char *path, const char *text, size_t len) {
  FILE *file = fopen(path, "wb");
  CHECK(file != NULL, "cannot create %s: %s", path, strerror(errno));
  if (file == NULL) return;
  CHECK(fwrite(text, 1, len, file) == len, "cannot write %s", path);
  fclose(file);
}

/* The whole file at `path` in a buffer the caller frees, or NULL. */
static char *read_file(const char *path, size_t *size) {
  struct stat info;
  FILE *file;
  char *data;

  *size = 0;
  if (stat(path, &info) != 0) return NULL;
  file = fopen(path, "rb");
  if (file == NULL) return NULL;
  data = malloc((size_t)info.st_size + 1);
  if (data != NULL) *size = fread(data, 1, (size_t)info.st_size, file);
  fclose(file);
  return data;
}

static size_t files_found;

static int count_file(const char *path, const struct stat *info, int flag, struct FTW *ftw) {
  (void)path, (void)info, (void)ftw;
  if (flag == FTW_F) files_found++;
  return 0;
}

static size_t files_under(const char *dir) {
  files_found = 0;
  CHECK(nftw(dir, count_file, 16, FTW_PHYS) == 0, "cannot walk %s", dir);
  return files_found;
}

/* Run the vitri executable with its output going to `log`; its exit status,
 * or -1. */
static int run_vitri(char *const argv[], const char *log) {
  pid_t pid = fork();
  int status;

  if (pid < 0) return -1;
  if (pid == 0) {
    int fd = open(log, O_WRONLY | O_CREAT | O_TRUNC, 0666);
    if (fd >= 0) {
      dup2(fd, STDOUT_FILENO);
      dup2(fd, STDERR_FILENO);
    }
    execv(argv[0], argv);
    _exit(127);
  }
  while (waitpid(pid, &status, 0) < 0)
    if (errno != EINTR) return -1;
  return WIFEXITED(status) ? WEXITSTATUS(status) : -1;
}

/* ---------------------------------------------------------- bundle checks */

/* The result holds exactly the files under `dir`, with the same bytes. */
static void expect_same_as_directory(const char *what, const vitri_result *result,
                                     const char *dir) {
  size_t count = vitri_result_file_count(result);

  CHECK(count == files_under(dir), "%s: %zu files in memory, %zu under %s", what, count,
        files_under(dir), dir);
  for (size_t i = 0; i < count; i++) {
    size_t path_len = 0, len = 0, disk_len;
    const char *path = vitri_result_file_path(result, i, &path_len);
    const uint8_t *contents = vitri_result_file_contents(result, i, &len);
    char full[2 * PATH_SIZE];
    char *disk;

    CHECK(path != NULL && contents != NULL, "%s: file %zu is missing", what, i);
    if (path == NULL || contents == NULL) continue;
    CHECK(path[path_len] == '\0' && strlen(path) == path_len,
          "%s: path %zu is not followed by a NUL at its length", what, i);
    CHECK(contents[len] == '\0', "%s: %s is not followed by a NUL", what, path);
    snprintf(full, sizeof full, "%s/%s", dir, path);
    disk = read_file(full, &disk_len);
    CHECK(disk != NULL, "%s: vitri -o wrote no %s", what, path);
    if (disk == NULL) continue;
    CHECK(disk_len == len && memcmp(disk, contents, len) == 0,
          "%s: %s differs from vitri -o (%zu bytes in memory, %zu on disk)", what, path,
          len, disk_len);
    free(disk);
  }
}

/* Two results hold the same summary and the same files. */
static void expect_same_bundle(const char *what, const vitri_result *a, const vitri_result *b) {
  size_t a_len = 0, b_len = 0;
  const char *a_summary = vitri_result_summary_json(a, &a_len);
  const char *b_summary = vitri_result_summary_json(b, &b_len);
  size_t count = vitri_result_file_count(a);

  CHECK(a_summary && b_summary && a_len == b_len && memcmp(a_summary, b_summary, a_len) == 0,
        "%s: the summaries differ:\n  %s\n  %s", what, or_null(a_summary), or_null(b_summary));
  CHECK(count == vitri_result_file_count(b), "%s: %zu files against %zu", what, count,
        vitri_result_file_count(b));
  if (count != vitri_result_file_count(b)) return;
  for (size_t i = 0; i < count; i++) {
    const char *a_path = vitri_result_file_path(a, i, &a_len);
    const char *b_path = vitri_result_file_path(b, i, &b_len);
    const uint8_t *a_bytes, *b_bytes;

    CHECK(a_len == b_len && memcmp(a_path, b_path, a_len) == 0, "%s: file %zu is %s and %s",
          what, i, a_path, b_path);
    a_bytes = vitri_result_file_contents(a, i, &a_len);
    b_bytes = vitri_result_file_contents(b, i, &b_len);
    CHECK(a_len == b_len && memcmp(a_bytes, b_bytes, a_len) == 0, "%s: %s differs", what,
          a_path);
  }
}

static int summary_mentions(const vitri_result *result, const char *needle) {
  const char *summary = vitri_result_summary_json(result, NULL);
  return summary != NULL && strstr(summary, needle) != NULL;
}

/* Prepare `dimacs` through the API with `request`, and through the executable
 * with `--vtree minfill-primal` and `flags`, and compare. The caller frees the
 * result. */
static vitri_result *prepare_like_vitri(const char *name, const char *dimacs, size_t len,
                                        const char *request, const char *const flags[]) {
  char cnf[PATH_SIZE], out[PATH_SIZE], log[PATH_SIZE];
  char *argv[16];
  int argc = 0;
  vitri_result *result = NULL;
  vitri_code code;

  snprintf(cnf, sizeof cnf, "%s/%s.cnf", scratch, name);
  snprintf(out, sizeof out, "%s/%s-vitri", scratch, name);
  snprintf(log, sizeof log, "%s/%s-vitri.log", scratch, name);
  write_text(cnf, dimacs, len);
  argv[argc++] = (char *)vitri_executable;
  argv[argc++] = (char *)"-o";
  argv[argc++] = out;
  argv[argc++] = (char *)"--vtree";
  argv[argc++] = (char *)"minfill-primal";
  for (int i = 0; flags[i] != NULL; i++) argv[argc++] = (char *)flags[i];
  argv[argc++] = cnf;
  argv[argc] = NULL;
  CHECK(run_vitri(argv, log) == 0, "%s: %s -o failed; see %s", name, vitri_executable, log);

  code = vitri_prepare((const uint8_t *)dimacs, len, request, strlen(request), &result);
  CHECK(code == VITRI_OK, "%s: code %d, %s: %s", name, code, kind_of(result),
        message_of(result));
  CHECK(vitri_result_code(result) == code, "%s: the result's code is not the returned one",
        name);
  expect_same_as_directory(name, result, out);
  return result;
}

/* A failed result: the code, the kind, a message mentioning `mentions`, and
 * none of a successful result's parts. Frees the result. */
static void expect_error(const char *what, vitri_code code, vitri_result *result,
                         vitri_code expected, const char *kind, const char *mentions) {
  size_t len = 1;
  const char *message;

  CHECK(code == expected, "%s: code %d, expected %d (%s: %s)", what, code, expected,
        kind_of(result), message_of(result));
  CHECK(result != NULL, "%s: no result", what);
  CHECK(vitri_result_code(result) == code, "%s: the result's code is not the returned one",
        what);
  CHECK(strcmp(kind_of(result), kind) == 0, "%s: kind %s, expected %s", what,
        kind_of(result), kind);
  message = vitri_result_error_message(result, &len);
  CHECK(message != NULL && len > 0 && strlen(message) == len,
        "%s: the message is missing or its length is wrong", what);
  CHECK(mentions == NULL || strstr(message_of(result), mentions) != NULL,
        "%s: the message does not mention %s: %s", what, mentions, message_of(result));
  CHECK(vitri_result_summary_json(result, &len) == NULL && len == 0, "%s: a summary", what);
  CHECK(vitri_result_status(result, &len) == NULL && len == 0, "%s: a status", what);
  CHECK(vitri_result_file_count(result) == 0, "%s: files", what);
  CHECK(vitri_result_file_path(result, 0, &len) == NULL && len == 0, "%s: a path", what);
  vitri_result_free(result);
}

/* ------------------------------------------------------------------ tests */

static void test_versions(void) {
  CHECK(vitri_abi_version() == VITRI_ABI_VERSION, "library ABI %u, header ABI %u",
        vitri_abi_version(), (unsigned)VITRI_ABI_VERSION);
  CHECK(strlen(vitri_version()) > 0, "no version string");
  CHECK(vitri_version() == vitri_version(), "the version string is not static");
}

static void test_capabilities(void) {
  size_t len = 0;
  char *json = vitri_capabilities_json(&len);

  CHECK(json != NULL && strlen(json) == len, "the capabilities are missing or mis-sized");
  CHECK(json && strstr(json, "\"format\":\"vitri-capabilities-v1\""),
        "the capabilities are not tagged: %s", or_null(json));
  CHECK(json && strstr(json, vitri_version()), "the capabilities do not carry the version");
  CHECK(json && strstr(json, "\"mode_stages\""), "the capabilities list no stages per mode");
  vitri_string_free(json);
  json = vitri_capabilities_json(NULL);
  CHECK(json != NULL, "the capabilities need a length pointer");
  vitri_string_free(json);
  vitri_string_free(NULL);
}

static volatile sig_atomic_t children_exited = 0;

static void on_child_exit(int signal) {
  (void)signal;
  children_exited = 1;
}

/* A host handler that waits for every child, as some event loops do. */
static void reap_every_child(int signal) {
  int saved = errno;
  (void)signal;
  while (waitpid(-1, NULL, WNOHANG) > 0) children_exited = 1;
  errno = saved;
}

/* Run IRREDUCIBLE with `request` under a SIGCHLD disposition, and expect the
 * Arjun stage to have run and the bundle to equal `reference`. */
static void expect_arjun_under(const char *what, const struct sigaction *disposition,
                               const char *request, const vitri_result *reference) {
  struct sigaction previous;
  vitri_result *result = NULL;
  vitri_code code;

  sigaction(SIGCHLD, disposition, &previous);
  code = prepare(IRREDUCIBLE, request, &result);
  sigaction(SIGCHLD, &previous, NULL);
  CHECK(code == VITRI_OK, "%s: code %d, %s", what, code, message_of(result));
  CHECK(summary_mentions(result, "\"arjun\":\"ran\""), "%s: the Arjun stage did not run: %s",
        what, or_null(vitri_result_summary_json(result, NULL)));
  expect_same_bundle(what, reference, result);
  vitri_result_free(result);
}

static pthread_mutex_t gate_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t gate_opened = PTHREAD_COND_INITIALIZER;
static int gate_open = 0;

static void *wait_at_gate(void *unused) {
  (void)unused;
  pthread_mutex_lock(&gate_lock);
  while (!gate_open) pthread_cond_wait(&gate_opened, &gate_lock);
  pthread_mutex_unlock(&gate_lock);
  return NULL;
}

/* With a budget, the Arjun stage forks in a process with one thread and runs
 * inline in a process with two, and both answer with the same bundle. A tiny
 * budget still answers. */
static void test_where_the_arjun_stage_runs(void) {
  static const char BUDGETED[] = "{\"vtree\": \"minfill-primal\", \"budget_ms\": 600000}";
  static const char TINY[] = "{\"vtree\": \"minfill-primal\", \"budget_ms\": 1}";
  struct sigaction action, previous;
  vitri_result *alone = NULL, *beside = NULL, *tiny = NULL;
  vitri_code code;
  pthread_t thread;

  memset(&action, 0, sizeof action);
  action.sa_handler = on_child_exit;
  sigemptyset(&action.sa_mask);
  action.sa_flags = SA_RESTART;
  sigaction(SIGCHLD, &action, &previous);

  children_exited = 0;
  code = prepare(IRREDUCIBLE, BUDGETED, &alone);
  CHECK(code == VITRI_OK, "one thread: code %d, %s", code, message_of(alone));
  CHECK(summary_mentions(alone, "\"arjun\":\"ran\""), "one thread: the Arjun stage did not run: %s",
        or_null(vitri_result_summary_json(alone, NULL)));
  CHECK(children_exited, "one thread: the Arjun stage ran without a child process");

  gate_open = 0;
  CHECK(pthread_create(&thread, NULL, wait_at_gate, NULL) == 0, "cannot start a thread");
  children_exited = 0;
  code = prepare(IRREDUCIBLE, BUDGETED, &beside);
  CHECK(code == VITRI_OK, "two threads: code %d, %s", code, message_of(beside));
  CHECK(!children_exited, "two threads: the Arjun stage ran in a child process");
  code = prepare(IRREDUCIBLE, TINY, &tiny);
  CHECK(code == VITRI_OK, "two threads, 1 ms: code %d, %s", code, message_of(tiny));
  vitri_result_free(tiny);
  pthread_mutex_lock(&gate_lock);
  gate_open = 1;
  pthread_cond_signal(&gate_opened);
  pthread_mutex_unlock(&gate_lock);
  pthread_join(thread, NULL);

  expect_same_bundle("forked against inline Arjun", alone, beside);
  vitri_result_free(beside);

  /* Children the kernel reaps on exit: the stage runs inline. A handler that
   * reaps every child: the stage still delivers its reduction. */
  memset(&action, 0, sizeof action);
  sigemptyset(&action.sa_mask);
  action.sa_handler = SIG_IGN;
  expect_arjun_under("SIGCHLD ignored", &action, BUDGETED, alone);
  action.sa_handler = on_child_exit;
  action.sa_flags = SA_NOCLDWAIT | SA_RESTART;
  expect_arjun_under("SA_NOCLDWAIT", &action, BUDGETED, alone);
  action.sa_handler = reap_every_child;
  action.sa_flags = SA_RESTART;
  expect_arjun_under("a handler that reaps every child", &action, BUDGETED, alone);
  vitri_result_free(alone);

  code = prepare(IRREDUCIBLE, TINY, &tiny);
  CHECK(code == VITRI_OK, "one thread, 1 ms: code %d, %s", code, message_of(tiny));
  vitri_result_free(tiny);

  sigaction(SIGCHLD, &previous, NULL);
}

static void test_bundles_match_the_executable(int count, char **cnfs) {
  vitri_result *result;

  size_t len = 0;
  const char *path;

  result = prepare_like_vitri("two-components", TWO_COMPONENTS, strlen(TWO_COMPONENTS),
                              UNPREPROCESSED, UNPREPROCESSED_FLAGS);
  CHECK(summary_mentions(result, "\"status\":\"built\""), "two-components: not built");
  path = NULL;
  for (size_t i = 0; i < vitri_result_file_count(result) && path == NULL; i++) {
    const char *candidate = vitri_result_file_path(result, i, &len);
    if (strncmp(candidate, "components/", 11) == 0) path = candidate;
  }
  CHECK(path != NULL, "two-components: no per-component files");
  vitri_result_free(result);

  result = prepare_like_vitri("irreducible", IRREDUCIBLE, strlen(IRREDUCIBLE), MINFILL,
                              MINFILL_FLAGS);
  vitri_result_free(result);

  for (int i = 0; i < count; i++) {
    char *dimacs = read_file(cnfs[i], &len);
    char name[64];

    CHECK(dimacs != NULL, "cannot read %s", cnfs[i]);
    if (dimacs == NULL) continue;
    snprintf(name, sizeof name, "argument-%d", i);
    /* The buffer ends at `len`, with no NUL after it. */
    result = prepare_like_vitri(name, dimacs, len, MINFILL_DOT, MINFILL_DOT_FLAGS);
    vitri_result_free(result);
    free(dimacs);
  }
}

static void test_runs_without_a_vtree(void) {
  static const struct {
    const char *name, *dimacs, *status;
  } cases[] = {
      {"fully-resolved", FULLY_RESOLVED, "fully_resolved"},
      {"refuted", REFUTED, "refuted"},
  };

  for (size_t c = 0; c < sizeof cases / sizeof cases[0]; c++) {
    vitri_result *result = prepare_like_vitri(cases[c].name, cases[c].dimacs,
                                              strlen(cases[c].dimacs), MINFILL_DOT,
                                              MINFILL_DOT_FLAGS);
    size_t len = 0;
    const char *status = vitri_result_status(result, &len);

    CHECK(status && strlen(cases[c].status) == len && strcmp(status, cases[c].status) == 0,
          "%s: status %s", cases[c].name, or_null(status));
    CHECK(summary_mentions(result, "\"vtree\":null"), "%s: the summary reports a vtree",
          cases[c].name);
    CHECK(vitri_result_file_count(result) == 2, "%s: %zu files", cases[c].name,
          vitri_result_file_count(result));
    for (size_t i = 0; i < vitri_result_file_count(result); i++) {
      const char *path = vitri_result_file_path(result, i, &len);
      CHECK(len < 6 || strcmp(path + len - 6, ".vtree") != 0, "%s: a vtree file %s",
            cases[c].name, path);
    }
    vitri_result_free(result);
  }
}

static void test_errors(void) {
  vitri_result *result = NULL;
  vitri_code code;
  /* A request that holds a NUL inside its length. */
  static const char NUL_INSIDE[] = "{}\0";

  code = prepare(MALFORMED, MINFILL, &result);
  expect_error("malformed DIMACS", code, result, VITRI_ERROR_INPUT, "input", NULL);

  code = prepare(IRREDUCIBLE, "not json", &result);
  expect_error("not JSON", code, result, VITRI_ERROR_CONFIG, "config", "not JSON");

  code = prepare(IRREDUCIBLE, "{\"threads\": 2}", &result);
  expect_error("an unknown key", code, result, VITRI_ERROR_CONFIG, "config", "\"threads\"");

  code = prepare(IRREDUCIBLE, "{\"format\": \"vitri-request-v2\"}", &result);
  expect_error("another format", code, result, VITRI_ERROR_CONFIG, "config",
               "vitri-request-v2");

  code = prepare(IRREDUCIBLE, "{\"mode\": \"count\"}", &result);
  expect_error("an unknown mode", code, result, VITRI_ERROR_CONFIG, "config", "\"count\"");

  code = prepare(IRREDUCIBLE, "{\"vtree\": \"minfill-primal\", \"candidates\": 2}", &result);
  expect_error("candidates with one vtree", code, result, VITRI_ERROR_CONFIG, "config",
               "candidates");

  code = prepare(IRREDUCIBLE, "{\"vtree\": \"no-such-construction\"}", &result);
  expect_error("an unknown construction", code, result, VITRI_ERROR_SPEC, "spec",
               "no-such-construction");

  code = prepare(IRREDUCIBLE, "{\"vtree\": \"\xff\"}", &result);
  expect_error("a request that is not UTF-8", code, result, VITRI_ERROR_CONFIG, "config",
               "UTF-8");

  code = vitri_prepare((const uint8_t *)IRREDUCIBLE, strlen(IRREDUCIBLE), NUL_INSIDE,
                       sizeof NUL_INSIDE - 1, &result);
  expect_error("a NUL inside the request", code, result, VITRI_ERROR_CONFIG, "config", NULL);
}

/* A stage switch under a mode whose preprocessing lacks that stage is refused,
 * by request key. The capabilities say which stages `compile` has. */
static void test_a_stage_the_mode_lacks_is_refused(void) {
  static const char REQUEST[] =
      "{\"mode\": \"compile\", \"vtree\": \"minfill-primal\", \"arjun\": false}";
  char *json = vitri_capabilities_json(NULL);
  const char *compile = json ? strstr(json, "\"compile\":{") : NULL;
  const char *end = compile ? strchr(compile, '}') : NULL;
  const char *arjun = compile ? strstr(compile, "\"arjun\":false") : NULL;
  vitri_result *result = NULL;
  vitri_code code;

  CHECK(end != NULL && arjun != NULL && arjun < end,
        "the capabilities do not list compile without an Arjun stage: %s", or_null(json));
  vitri_string_free(json);

  code = prepare(IRREDUCIBLE, REQUEST, &result);
  CHECK(strstr(message_of(result), "--no-") == NULL,
        "the refusal names a command-line flag: %s", message_of(result));
  expect_error("arjun under compile", code, result, VITRI_ERROR_CONFIG, "config",
               "arjun=false");
}

static void test_argument_contract(void) {
  vitri_result *result = NULL;
  vitri_code code;
  size_t len = 1;
  /* Buffers with more after the lengths given. */
  static const char REQUEST_AND_MORE[] = "{\"vtree\": \"minfill-primal\"}, ignored";
  char *dimacs = malloc(sizeof TWO_COMPONENTS - 1);

  code = prepare(IRREDUCIBLE, MINFILL, NULL);
  CHECK(code == VITRI_ERROR_INVALID_ARGUMENT, "a null out: code %d", code);

  code = vitri_prepare(NULL, 5, MINFILL, strlen(MINFILL), &result);
  expect_error("null DIMACS with a length", code, result, VITRI_ERROR_INVALID_ARGUMENT,
               "argument", "dimacs");

  code = vitri_prepare((const uint8_t *)IRREDUCIBLE, strlen(IRREDUCIBLE), NULL, 3, &result);
  expect_error("a null request with a length", code, result, VITRI_ERROR_INVALID_ARGUMENT,
               "argument", "request");

  code = vitri_prepare((const uint8_t *)IRREDUCIBLE, SIZE_MAX, MINFILL, strlen(MINFILL),
                       &result);
  expect_error("a DIMACS length past any buffer", code, result, VITRI_ERROR_INVALID_ARGUMENT,
               "argument", "dimacs_len");

  code = vitri_prepare(NULL, 0, MINFILL, strlen(MINFILL), &result);
  expect_error("null DIMACS of length 0", code, result, VITRI_ERROR_INPUT, "input", NULL);

  code = vitri_prepare((const uint8_t *)IRREDUCIBLE, 0, MINFILL, strlen(MINFILL), &result);
  expect_error("DIMACS of length 0", code, result, VITRI_ERROR_INPUT, "input", NULL);

  code = vitri_prepare((const uint8_t *)IRREDUCIBLE, strlen(IRREDUCIBLE), MINFILL, 0, &result);
  expect_error("a request of length 0", code, result, VITRI_ERROR_CONFIG, "config", NULL);

  code = vitri_prepare((const uint8_t *)IRREDUCIBLE, strlen(IRREDUCIBLE), NULL, 0, &result);
  CHECK(code == VITRI_OK, "the default request: code %d, %s", code, message_of(result));
  CHECK(summary_mentions(result, "\"status\":\"built\""), "the default request: not built");
  CHECK(vitri_result_error_kind(result, &len) == NULL && len == 0,
        "a successful result has an error kind");
  CHECK(vitri_result_error_message(result, &len) == NULL && len == 0,
        "a successful result has an error message");
  len = 1;
  CHECK(vitri_result_file_path(result, vitri_result_file_count(result), &len) == NULL &&
            len == 0,
        "a path past the last file");
  len = 1;
  CHECK(vitri_result_file_contents(result, SIZE_MAX, &len) == NULL && len == 0,
        "contents past the last file");
  CHECK(vitri_result_status(result, NULL) != NULL, "a status needs a length pointer");
  CHECK(vitri_result_file_contents(result, 0, NULL) != NULL,
        "contents need a length pointer");
  vitri_result_free(result);

  /* Lengths are honored: nothing past them is read. */
  if (dimacs != NULL) {
    memcpy(dimacs, TWO_COMPONENTS, sizeof TWO_COMPONENTS - 1);
    code = vitri_prepare((const uint8_t *)dimacs, sizeof TWO_COMPONENTS - 1, REQUEST_AND_MORE,
                         strlen(MINFILL), &result);
    CHECK(code == VITRI_OK, "unterminated buffers: code %d, %s", code, message_of(result));
    vitri_result_free(result);
    free(dimacs);
  }

  len = 1;
  CHECK(vitri_result_code(NULL) == VITRI_ERROR_INVALID_ARGUMENT, "the code of null");
  CHECK(vitri_result_status(NULL, &len) == NULL && len == 0, "the status of null");
  len = 1;
  CHECK(vitri_result_summary_json(NULL, &len) == NULL && len == 0, "the summary of null");
  CHECK(vitri_result_file_count(NULL) == 0, "the file count of null");
  len = 1;
  CHECK(vitri_result_file_path(NULL, 0, &len) == NULL && len == 0, "a path of null");
  len = 1;
  CHECK(vitri_result_file_contents(NULL, 0, &len) == NULL && len == 0, "contents of null");
  len = 1;
  CHECK(vitri_result_error_kind(NULL, &len) == NULL && len == 0, "the kind of null");
  len = 1;
  CHECK(vitri_result_error_message(NULL, &len) == NULL && len == 0, "the message of null");
  vitri_result_free(NULL);
}

enum { REPEATS = 20, THREADS = 4 };

static void test_repeated_calls(void) {
  vitri_result *first = NULL;
  vitri_code code = prepare(TWO_COMPONENTS, UNPREPROCESSED, &first);

  CHECK(code == VITRI_OK, "the first call: code %d", code);
  for (int i = 0; i < REPEATS; i++) {
    vitri_result *again = NULL, *failed = NULL;

    code = prepare(TWO_COMPONENTS, UNPREPROCESSED, &again);
    CHECK(code == VITRI_OK, "call %d: code %d", i, code);
    expect_same_bundle("a repeated call", first, again);
    vitri_result_free(again);
    code = prepare(MALFORMED, MINFILL, &failed);
    expect_error("a repeated failure", code, failed, VITRI_ERROR_INPUT, "input", NULL);
  }
  vitri_result_free(first);
}

struct call {
  pthread_t thread;
  vitri_code code;
  vitri_result *result;
};

static void *call_prepare(void *arg) {
  struct call *call = arg;
  call->code = prepare(TWO_COMPONENTS, UNPREPROCESSED, &call->result);
  return NULL;
}

static void test_concurrent_calls(void) {
  vitri_result *reference = NULL;
  struct call calls[THREADS];

  CHECK(prepare(TWO_COMPONENTS, UNPREPROCESSED, &reference) == VITRI_OK, "the reference call");
  for (int i = 0; i < THREADS; i++) {
    calls[i].result = NULL;
    CHECK(pthread_create(&calls[i].thread, NULL, call_prepare, &calls[i]) == 0,
          "cannot start thread %d", i);
  }
  for (int i = 0; i < THREADS; i++) {
    pthread_join(calls[i].thread, NULL);
    CHECK(calls[i].code == VITRI_OK, "thread %d: code %d, %s", i, calls[i].code,
          message_of(calls[i].result));
    expect_same_bundle("a concurrent call", reference, calls[i].result);
    vitri_result_free(calls[i].result);
  }
  vitri_result_free(reference);
}

int main(int argc, char **argv) {
  if (argc < 3) {
    fprintf(stderr, "usage: %s VITRI_EXECUTABLE SCRATCH_DIR [CNF...]\n", argv[0]);
    return 2;
  }
  vitri_executable = argv[1];
  scratch = argv[2];
  if (mkdir(scratch, 0777) != 0 && errno != EEXIST) {
    perror(scratch);
    return 2;
  }

  /* First, while the process still has one thread. */
  test_where_the_arjun_stage_runs();
  test_versions();
  test_capabilities();
  test_bundles_match_the_executable(argc - 3, argv + 3);
  test_runs_without_a_vtree();
  test_errors();
  test_a_stage_the_mode_lacks_is_refused();
  test_argument_contract();
  test_repeated_calls();
  test_concurrent_calls();

  if (failures) {
    printf("%d checks failed\n", failures);
    return 1;
  }
  printf("all checks passed\n");
  return 0;
}
