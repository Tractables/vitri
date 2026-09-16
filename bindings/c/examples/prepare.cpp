// Prepares a CNF with vitri, writes the bundle into a directory as
// `vitri -o OUT_DIR` does, and prints the run's summary: the C example in
// C++17, with the result held by a std::unique_ptr.
//
//   prepare INPUT.cnf OUT_DIR [REQUEST_JSON]
//
// Building and linking this program is covered in bindings/c/README.md.

#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <iterator>
#include <memory>
#include <string>
#include <string_view>

#include "vitri.h"

namespace {

struct ResultFree {
  void operator()(vitri_result *result) const { vitri_result_free(result); }
};

using Result = std::unique_ptr<vitri_result, ResultFree>;

// Each accessor lends a buffer that stays valid while the result is alive;
// these wrap one call as a view. The length is read after the call returns.
using Accessor = const char *(const vitri_result *, std::size_t *);

std::string_view view(const Result &result, Accessor *accessor) {
  std::size_t len = 0;
  const char *data = accessor(result.get(), &len);
  return {data, len};
}

std::string_view file_path(const Result &result, std::size_t index) {
  std::size_t len = 0;
  const char *data = vitri_result_file_path(result.get(), index, &len);
  return {data, len};
}

std::string_view file_contents(const Result &result, std::size_t index) {
  std::size_t len = 0;
  const std::uint8_t *data = vitri_result_file_contents(result.get(), index, &len);
  return {reinterpret_cast<const char *>(data), len};
}

}  // namespace

int main(int argc, char **argv) {
  if (argc != 3 && argc != 4) {
    std::cerr << "usage: " << argv[0] << " INPUT.cnf OUT_DIR [REQUEST_JSON]\n";
    return 2;
  }
  if (vitri_abi_version() != VITRI_ABI_VERSION) {
    std::cerr << "the vitri library has ABI " << vitri_abi_version()
              << ", this program was built for " << VITRI_ABI_VERSION << '\n';
    return 1;
  }

  std::ifstream input(argv[1], std::ios::binary);
  if (!input) {
    std::cerr << argv[1] << ": cannot open\n";
    return 1;
  }
  const std::string dimacs{std::istreambuf_iterator<char>(input), {}};
  // Without a request argument, a null pointer asks for every default.
  const std::string_view request = argc == 4 ? argv[3] : std::string_view{};

  vitri_result *raw = nullptr;
  const vitri_code code = vitri_prepare(
      reinterpret_cast<const std::uint8_t *>(dimacs.data()), dimacs.size(),
      argc == 4 ? request.data() : nullptr, request.size(), &raw);
  const Result result{raw};

  if (code != VITRI_OK) {
    std::cerr << "vitri " << view(result, vitri_result_error_kind)
              << " error: " << view(result, vitri_result_error_message) << '\n';
    return 1;
  }

  const std::filesystem::path out_dir = argv[2];
  for (std::size_t i = 0; i < vitri_result_file_count(result.get()); ++i) {
    const std::string_view contents = file_contents(result, i);
    const std::filesystem::path target = out_dir / std::filesystem::path(file_path(result, i));
    std::filesystem::create_directories(target.parent_path());
    std::ofstream file(target, std::ios::binary);
    file.write(contents.data(), static_cast<std::streamsize>(contents.size()));
    if (!file.flush()) {
      std::cerr << target.string() << ": cannot write\n";
      return 1;
    }
  }

  std::cout << view(result, vitri_result_summary_json) << '\n';
  return 0;
}
