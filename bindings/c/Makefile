# Builds the C bindings and the programs that use them, on Linux. Run from
# bindings/c; README.md describes each target.
#
#   make                 the shared and static libraries, and the vitri executable
#   make check           examples and tests against each library
#   make valgrind        the tests under valgrind, failing on leaks and memory errors
#   make linkage         GMP, MPFR and the C++ runtime stay dynamic dependencies
#   make link-libs-check Libs.private in vitri-c.pc.in covers what rustc reports
#   make header          regenerate include/vitri.h
#   make header-check    fail if include/vitri.h differs from cbindgen's output

SHELL := bash
.SHELLFLAGS := -eo pipefail -c

CARGO ?= cargo
CBINDGEN ?= cbindgen
CC ?= cc
CXX ?= c++
VALGRIND ?= valgrind
CFLAGS ?= -O2 -g
CXXFLAGS ?= -O2 -g

RELEASE := target/release
OUT := target/c
SHARED := $(RELEASE)/libvitri_c.so
STATIC := $(RELEASE)/libvitri_c.a
VITRI := $(RELEASE)/vitri

# The system libraries a static link needs. vitri-c.pc.in is the one place
# they are listed.
STATIC_LIBS := $(shell sed -n 's/^Libs.private: //p' vitri-c.pc.in)
LINK_shared := -L$(RELEASE) -lvitri_c -Wl,-rpath,$(abspath $(RELEASE))
LINK_static := $(STATIC) $(STATIC_LIBS)

WARNINGS := -Wall -Wextra -Werror
EXAMPLE_CNF := ../../docs/example.cnf
EXAMPLE_REQUEST := {"vtree": "minfill-primal", "dot": true}
KINDS := shared static
PROGRAMS := $(foreach kind,$(KINDS),$(OUT)/prepare-c-$(kind) $(OUT)/prepare-cpp-$(kind) $(OUT)/test-$(kind))

.PHONY: all libs check valgrind linkage link-libs-check header header-check clean

all: libs

# rustc prints the static link line only when it compiles this crate, so the
# crate is touched to make every run print it.
libs:
	@mkdir -p $(OUT)
	touch src/lib.rs
	$(CARGO) rustc --release --lib -- --print native-static-libs 2>&1 | tee $(OUT)/rustc.log
	$(CARGO) build --release -p vitri --bin vitri

$(OUT)/prepare-c-%: examples/prepare.c include/vitri.h $(SHARED) $(STATIC)
	$(CC) -std=c99 $(WARNINGS) $(CFLAGS) -Iinclude $< $(LINK_$*) -o $@

$(OUT)/prepare-cpp-%: examples/prepare.cpp include/vitri.h $(SHARED) $(STATIC)
	$(CXX) -std=c++17 $(WARNINGS) $(CXXFLAGS) -Iinclude $< $(LINK_$*) -o $@

$(OUT)/test-%: tests/test_vitri.c include/vitri.h $(SHARED) $(STATIC)
	$(CC) -std=c99 $(WARNINGS) $(CFLAGS) -pthread -Iinclude $< $(LINK_$*) -o $@

# The examples must write the bundle vitri -o writes; the test driver checks
# the rest.
check: $(PROGRAMS)
	rm -rf $(OUT)/run && mkdir -p $(OUT)/run
	$(VITRI) -o $(OUT)/run/vitri --vtree minfill-primal --dot $(EXAMPLE_CNF) > $(OUT)/run/vitri.log
	for kind in $(KINDS); do \
	  for lang in c cpp; do \
	    $(OUT)/prepare-$$lang-$$kind $(EXAMPLE_CNF) $(OUT)/run/$$lang-$$kind '$(EXAMPLE_REQUEST)' \
	      > $(OUT)/run/$$lang-$$kind.json; \
	    diff -r $(OUT)/run/vitri $(OUT)/run/$$lang-$$kind; \
	  done; \
	  $(OUT)/test-$$kind $(VITRI) $(OUT)/run/test-$$kind $(EXAMPLE_CNF); \
	done

valgrind: $(OUT)/test-shared $(OUT)/test-static
	rm -rf $(OUT)/valgrind && mkdir -p $(OUT)/valgrind
	for kind in $(KINDS); do \
	  $(VALGRIND) --error-exitcode=99 --leak-check=full \
	    --errors-for-leak-kinds=definite,indirect --child-silent-after-fork=yes \
	    $(OUT)/test-$$kind $(VITRI) $(OUT)/valgrind/$$kind $(EXAMPLE_CNF); \
	done

# The shared library, and a program linked against the static one, each load
# GMP and the C++ runtime from the system. Neither defines a GMP or MPFR
# function; any MPFR function they call comes from the system library too.
# (The linker drops a library nothing calls, so MPFR need not be loaded.)
linkage: $(OUT)/prepare-c-shared $(OUT)/prepare-c-static
	for file in $(SHARED) $(OUT)/prepare-c-static; do \
	  needed=$$(readelf -d $$file | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); \
	  echo "$$file needs:" $$needed; \
	  for lib in libgmp.so libstdc++.so; do \
	    grep -q "^$$lib" <<< "$$needed" || { echo "$$file does not load $$lib"; exit 1; }; \
	  done; \
	  if nm -D --undefined-only $$file | grep -q ' mpfr_'; then \
	    grep -q '^libmpfr.so' <<< "$$needed" || { echo "$$file calls MPFR without loading it"; exit 1; }; \
	  fi; \
	  if nm --defined-only $$file 2>/dev/null | grep -Eq ' [A-Za-z] (__gmp[a-z]*_|mpfr_)'; then \
	    echo "$$file defines GMP or MPFR functions"; exit 1; \
	  fi; \
	  if nm -D --defined-only $$file | grep -Eq ' [A-Za-z] (__gmp[a-z]*_|mpfr_)'; then \
	    echo "$$file exports GMP or MPFR functions"; exit 1; \
	  fi; \
	done
	readelf -d $(OUT)/prepare-c-shared | grep -q '(NEEDED).*\[libvitri_c.so\]' \
	  || { echo "$(OUT)/prepare-c-shared does not load libvitri_c.so"; exit 1; }

link-libs-check:
	libs=$$(sed -n 's/^note: native-static-libs: //p' $(OUT)/rustc.log | head -n 1); \
	test -n "$$libs" || { echo "no native-static-libs note in $(OUT)/rustc.log; run make libs"; exit 1; }; \
	echo "rustc: $$libs"; \
	missing=; \
	for lib in $$libs; do \
	  case " $(STATIC_LIBS) " in *" $$lib "*) ;; *) missing="$$missing $$lib" ;; esac; \
	done; \
	test -z "$$missing" || { echo "Libs.private in vitri-c.pc.in lacks:$$missing"; exit 1; }

header:
	$(CBINDGEN) --config cbindgen.toml --crate vitri-c --output include/vitri.h .

header-check:
	@mkdir -p $(OUT)
	$(CBINDGEN) --config cbindgen.toml --crate vitri-c --output $(OUT)/vitri.h .
	diff -u include/vitri.h $(OUT)/vitri.h || { \
	  echo "include/vitri.h is out of date; regenerate it with make header"; exit 1; }

clean:
	rm -rf $(OUT)
