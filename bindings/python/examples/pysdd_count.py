"""Count the models of a DIMACS CNF with PySDD over the vtree vitri builds.

PySDD compiles the reduced formula over vitri's vtree, and the lift in the
summary turns that count into the count of the input formula. PySDD accepts
any vtree; a compiler that requires a decision vtree, such as miniC2D, cannot
be given this one. This example handles unweighted model counting only.

    python -m pip install pysdd
    python pysdd_count.py formula.cnf
"""

import os
import sys
import tempfile
from fractions import Fraction

from pysdd.sdd import SddManager, Vtree

import vitri


def count_models(dimacs, **settings):
    """The number of models of `dimacs`, over every variable it declares."""
    result = vitri.prepare(dimacs, mode="mc", **settings)
    if result.status == "refuted":
        return 0
    if result.status == "fully_resolved":
        # Every variable was settled, so the reduced formula has one model.
        reduced_count = 1
    else:
        # PySDD reads both inputs from files.
        with tempfile.TemporaryDirectory() as directory:
            result.write(directory)
            vtree = Vtree.from_file(os.path.join(directory, "vtree.vtree").encode())
            manager = SddManager.from_vtree(vtree)
            circuit = manager.read_cnf_file(
                os.path.join(directory, "reduced.cnf").encode()
            )
            # Over every variable of the vtree, not only those the circuit
            # mentions.
            reduced_count = circuit.global_model_count()
    lift = result.summary["lift"]
    count = (
        reduced_count * 2 ** lift["count_lift_pow2"] * Fraction(lift["weight_lift"])
    )
    assert count.denominator == 1
    return count.numerator


if __name__ == "__main__":
    with open(sys.argv[1], "rb") as stream:
        print(count_models(stream.read()))
