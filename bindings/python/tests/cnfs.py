"""Formulas the tests share, and a brute-force model counter to check against."""

import itertools
import pathlib

PACKAGE = pathlib.Path(__file__).resolve().parents[1]
REPOSITORY = PACKAGE.parents[1]
EXAMPLES = PACKAGE / "examples"

# The formula the getting-started guide counts, which has fifty models.
EXAMPLE = (REPOSITORY / "docs" / "example.cnf").read_bytes()

# Two components of three variables each.
TWO_COMPONENTS = "p cnf 6 6\n1 2 3 0\n-1 -2 0\n-2 -3 0\n4 5 6 0\n-4 -5 0\n-5 -6 0\n"

# Variable 7 is forced, and variables 8 and 9 appear in no clause, so the count
# of the reduced formula lifts back by a power of two.
LIFTED = "p cnf 9 6\n1 2 3 0\n-1 -2 0\n-2 -3 0\n4 -5 6 0\n-4 5 0\n7 0\n"

REFUTED = "p cnf 1 2\n1 0\n-1 0\n"

# Every variable is forced or free, so nothing is left to build a vtree over.
FULLY_RESOLVED = "p cnf 3 2\n1 0\n-1 2 0\n"

MALFORMED = "p cnf 2 1\n1 x 0\n"


def brute_force_count(dimacs):
    """The number of assignments to the declared variables that satisfy every
    clause, by enumeration."""
    if isinstance(dimacs, bytes):
        dimacs = dimacs.decode()
    variables = 0
    clauses = []
    clause = []
    for line in dimacs.splitlines():
        fields = line.split()
        if not fields or fields[0] == "c":
            continue
        if fields[0] == "p":
            variables = int(fields[2])
            continue
        for literal in map(int, fields):
            if literal == 0:
                clauses.append(clause)
                clause = []
            else:
                clause.append(literal)
    return sum(
        all(any(values[abs(lit) - 1] == (lit > 0) for lit in c) for c in clauses)
        for values in itertools.product((False, True), repeat=variables)
    )
