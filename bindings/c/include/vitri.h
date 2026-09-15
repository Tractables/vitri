/* vitri: CNF preprocessing and vtree construction.
 *
 * Copyright the vitri authors. Licensed under the Apache License, Version 2.0.
 *
 * Compare VITRI_ABI_VERSION with vitri_abi_version() at run time to catch a
 * library built from a different ABI than this header.
 */

#ifndef VITRI_H
#define VITRI_H

/* Generated from bindings/c/src/lib.rs by cbindgen. Do not edit by hand; CI
 * checks that this file still matches the source. */

#include <stddef.h>
#include <stdint.h>

/**
 * The version of the ABI this header declares. `vitri_abi_version` returns
 * the version of the library that is actually loaded. It is raised whenever
 * a signature, a status code or an ownership rule changes in a way that
 * breaks a program built against the previous header. The JSON documents
 * carry their own format tags.
 */
#define VITRI_ABI_VERSION 1

/**
 * What one `vitri_prepare` call produced: the files and summary of a run, or
 * the error that stopped it.
 *
 * Opaque. Read it with the `vitri_result_` functions and release it with
 * `vitri_result_free`. A result does not change after `vitri_prepare` returns
 * it, so several threads may read one at the same time; free it once, after
 * the last read.
 */
typedef struct vitri_result vitri_result;

/**
 * A status code: `VITRI_OK`, or one of the `VITRI_ERROR_` values.
 */
typedef int32_t vitri_code;

/**
 * The call succeeded.
 */
#define VITRI_OK 0

/**
 * Error kind `config`: the request is not a JSON object of known keys with
 * valid values, or it combines settings that do not work together, such as
 * a candidate count with a construction that builds one vtree.
 */
#define VITRI_ERROR_CONFIG 1

/**
 * Error kind `spec`: the `vtree` spec names no construction, or a parameter
 * the named construction cannot honor.
 */
#define VITRI_ERROR_SPEC 2

/**
 * Error kind `env`: an environment variable vitri reads holds a value it
 * cannot use.
 */
#define VITRI_ERROR_ENV 3

/**
 * Error kind `input`: the DIMACS text cannot be used.
 */
#define VITRI_ERROR_INPUT 4

/**
 * Error kind `mismatch`: two parts of one run describe different formulas.
 */
#define VITRI_ERROR_MISMATCH 5

/**
 * Error kind `construction`: a vtree construction could not answer a well
 * formed request. Another spec or a larger budget may succeed.
 */
#define VITRI_ERROR_CONSTRUCTION 6

/**
 * Error kind `io`: a file could not be read or written.
 */
#define VITRI_ERROR_IO 7

/**
 * An error kind added to vitri after this header was generated. The kind
 * string names it.
 */
#define VITRI_ERROR_OTHER 8

/**
 * Error kind `argument`: an argument broke this header's contract, such as a
 * null pointer with a nonzero length.
 */
#define VITRI_ERROR_INVALID_ARGUMENT 9

/**
 * Error kind `panic`: vitri panicked. The panic stopped at this boundary and
 * its message is the error message; report it as a bug.
 */
#define VITRI_ERROR_PANIC 10

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/**
 * The ABI version of the loaded library, which is the `VITRI_ABI_VERSION` it
 * was built with.
 */
uint32_t vitri_abi_version(void);

/**
 * The version of vitri inside the library, such as `0.2.0`, as a
 * NUL-terminated string. The string is static: the caller does not free it,
 * and it stays valid for as long as the library is loaded.
 */
const char *vitri_version(void);

/**
 * What this build accepts, as the JSON object tagged
 * `"format": "vitri-capabilities-v1"`: the request keys, the modes, component
 * policies and vtree spec bases, the candidate ceiling and the preprocessing
 * stages this build carries. The `vitri::request::Capabilities` documentation
 * defines each field.
 *
 * Returns a NUL-terminated string that the caller owns and releases with
 * `vitri_string_free`, and writes its length, not counting the NUL, to `*len`
 * when `len` is not null. Returns null, with `*len` set to 0, if vitri
 * panicked.
 *
 * # Safety
 *
 * `len` must be null or point to storage for one `size_t`.
 */
char *vitri_capabilities_json(size_t *len);

/**
 * Release a string returned by `vitri_capabilities_json`. Null is ignored.
 *
 * # Safety
 *
 * `text` must be null or a string `vitri_capabilities_json` returned that has
 * not been freed.
 */
void vitri_string_free(char *text);

/**
 * Run vitri over DIMACS text with the settings in a JSON request, and return
 * the bundle `vitri -o DIR` would write for the same input and settings, held
 * in memory.
 *
 * `dimacs` points to `dimacs_len` bytes of DIMACS CNF text; it need not end
 * in a NUL. It may be null when `dimacs_len` is 0.
 *
 * `request` points to `request_len` bytes of UTF-8 JSON, which need not end
 * in a NUL: an object with any of the keys `format`, `mode`, `vtree`,
 * `budget_ms`, `components`, `candidates`, `simplify`, `arjun` and `dot`. The
 * `vitri::request::Request` documentation defines them, and
 * `vitri_capabilities_json` lists them with their accepted values. A key left
 * out keeps its default; an unknown key is refused. A null `request` with
 * `request_len` 0 is the request with every key left out. A setting the
 * request leaves out takes the library default, whatever the environment
 * says; `docs/env.md` lists the `VITRI_*` variables a run still reads.
 *
 * Returns `VITRI_OK` or a `VITRI_ERROR_` code. Unless `out` is null, `*out`
 * receives a new result whatever the code, and the code equals
 * `vitri_result_code(*out)`: the bundle on success, the error kind and
 * message otherwise. The caller owns `*out` and releases it with
 * `vitri_result_free`. A null `out` returns `VITRI_ERROR_INVALID_ARGUMENT`
 * and produces no result. vitri keeps no reference to `dimacs` or `request`
 * after returning.
 *
 * Calls run one at a time: a call made while another thread's call is
 * running waits for it to finish.
 *
 * `budget_ms` is not a hard limit. vitri checks the deadline between its
 * stages and at points inside them, so a run can end after it. On Linux and
 * macOS, when the process has exactly one thread, the Arjun reduction of an
 * unprojected mode runs in a forked child, which is killed shortly after the
 * deadline if it is still running; in a process with more threads, and on
 * other platforms, it runs in the calling thread and only its own checks stop
 * it. vitri waits for that child itself, so a process that sets `SIGCHLD` to
 * `SIG_IGN` or reaps every child loses the reduction: the stage reports
 * `gave_up` in the summary. To stop a run at a hard limit, run it in a
 * separate process that can be killed, such as the `vitri` executable.
 *
 * # Safety
 *
 * `dimacs` must be null or point to `dimacs_len` readable bytes, `request`
 * must be null or point to `request_len` readable bytes, neither may change
 * during the call, and `out` must be null or point to storage for one
 * pointer.
 */
vitri_code vitri_prepare(const uint8_t *dimacs,
                         size_t dimacs_len,
                         const char *request,
                         size_t request_len,
                         struct vitri_result **out);

/**
 * Release a result. Null is ignored.
 *
 * # Safety
 *
 * `result` must be null or a result `vitri_prepare` produced that has not
 * been freed. Every buffer borrowed from it becomes invalid.
 */
void vitri_result_free(struct vitri_result *result);

/**
 * The code `vitri_prepare` returned with this result: `VITRI_OK` or a
 * `VITRI_ERROR_` value. `VITRI_ERROR_INVALID_ARGUMENT` for a null `result`.
 *
 * # Safety
 *
 * `result` must be null or a live result from `vitri_prepare`.
 */
vitri_code vitri_result_code(const struct vitri_result *result);

/**
 * How the run ended: `built` when it built a vtree, `fully_resolved` or
 * `refuted` when preprocessing settled the formula and there is no vtree to
 * build. Only a `built` result has vtree files. This is the `status` field of
 * the summary.
 *
 * Writes the string's length to `*len` when `len` is not null. The bytes
 * belong to `result`: they stay valid and unchanged until
 * `vitri_result_free(result)`, and are followed by a NUL that `*len` does not
 * count. Null, with `*len` set to 0, for a failed or null `result`.
 *
 * # Safety
 *
 * `result` must be null or a live result from `vitri_prepare`, and `len` must
 * be null or point to storage for one `size_t`.
 */
const char *vitri_result_status(const struct vitri_result *result, size_t *len);

/**
 * The run's summary: the JSON object tagged `"format": "vitri-result-v1"`,
 * with the run's status, the formula's size before and after preprocessing,
 * the count lift, what each stage did, the vtree's size, the settings after
 * defaults were applied, and the bundle's paths in order. The
 * `vitri::request::Summary` documentation defines each field.
 *
 * Writes the text's length to `*len` when `len` is not null. The bytes belong
 * to `result`: they stay valid and unchanged until
 * `vitri_result_free(result)`, and are followed by a NUL that `*len` does not
 * count. Null, with `*len` set to 0, for a failed or null `result`.
 *
 * # Safety
 *
 * `result` must be null or a live result from `vitri_prepare`, and `len` must
 * be null or point to storage for one `size_t`.
 */
const char *vitri_result_summary_json(const struct vitri_result *result, size_t *len);

/**
 * How many files the bundle holds. 0 for a failed or null `result`.
 *
 * # Safety
 *
 * `result` must be null or a live result from `vitri_prepare`.
 */
size_t vitri_result_file_count(const struct vitri_result *result);

/**
 * The path of file `index`, counting from 0, relative to the bundle
 * directory and `/`-separated, such as `reduced.cnf` or
 * `components/comp000.vtree`. The files come in the order the summary lists
 * them.
 *
 * Writes the path's length to `*len` when `len` is not null. The bytes belong
 * to `result`: they stay valid and unchanged until
 * `vitri_result_free(result)`, and are followed by a NUL that `*len` does not
 * count. Null, with `*len` set to 0, for a failed or null `result` or an
 * `index` not below `vitri_result_file_count(result)`.
 *
 * # Safety
 *
 * `result` must be null or a live result from `vitri_prepare`, and `len` must
 * be null or point to storage for one `size_t`.
 */
const char *vitri_result_file_path(const struct vitri_result *result, size_t index, size_t *len);

/**
 * The contents of file `index`, counting from 0: the bytes `vitri -o DIR`
 * writes to that file's path.
 *
 * Writes the length to `*len` when `len` is not null. The bytes belong to
 * `result`: they stay valid and unchanged until `vitri_result_free(result)`,
 * and are followed by a NUL that `*len` does not count. Null, with `*len` set
 * to 0, for a failed or null `result` or an `index` not below
 * `vitri_result_file_count(result)`; an empty file is a non-null pointer with
 * length 0.
 *
 * # Safety
 *
 * `result` must be null or a live result from `vitri_prepare`, and `len` must
 * be null or point to storage for one `size_t`.
 */
const uint8_t *vitri_result_file_contents(const struct vitri_result *result,
                                          size_t index,
                                          size_t *len);

/**
 * The error's kind as one lowercase word: `config`, `spec`, `env`, `input`,
 * `mismatch`, `construction` or `io` from vitri, `argument` for a broken
 * argument contract, or `panic`. Each has its own `VITRI_ERROR_` code; a kind
 * newer than this header comes with `VITRI_ERROR_OTHER`.
 *
 * Writes the word's length to `*len` when `len` is not null. The bytes belong
 * to `result`: they stay valid and unchanged until
 * `vitri_result_free(result)`, and are followed by a NUL that `*len` does not
 * count. Null, with `*len` set to 0, for a successful or null `result`.
 *
 * # Safety
 *
 * `result` must be null or a live result from `vitri_prepare`, and `len` must
 * be null or point to storage for one `size_t`.
 */
const char *vitri_result_error_kind(const struct vitri_result *result, size_t *len);

/**
 * The error message: UTF-8 text saying what went wrong, which names the
 * request key, setting or input line at fault where there is one.
 *
 * Writes the message's length to `*len` when `len` is not null. The bytes
 * belong to `result`: they stay valid and unchanged until
 * `vitri_result_free(result)`, and are followed by a NUL that `*len` does not
 * count. Null, with `*len` set to 0, for a successful or null `result`.
 *
 * # Safety
 *
 * `result` must be null or a live result from `vitri_prepare`, and `len` must
 * be null or point to storage for one `size_t`.
 */
const char *vitri_result_error_message(const struct vitri_result *result, size_t *len);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* VITRI_H */
