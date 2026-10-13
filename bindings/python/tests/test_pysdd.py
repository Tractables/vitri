"""examples/pysdd_count.py: PySDD compiles the reduced formula over vitri's
vtree, and the lift turns its count into the input's. Runs when PySDD is
installed."""

import re
import subprocess
import sys

import pytest

import vitri
from cnfs import (
    EXAMPLE,
    EXAMPLES,
    FULLY_RESOLVED,
    LIFTED,
    REFUTED,
    TWO_COMPONENTS,
    brute_force_count,
)

pytest.importorskip("pysdd")
sys.path.insert(0, str(EXAMPLES))
import pysdd_count  # noqa: E402

FORMULAS = {
    "example": EXAMPLE,
    "two-components": TWO_COMPONENTS,
    "lifted": LIFTED,
    "fully-resolved": FULLY_RESOLVED,
    "refuted": REFUTED,
}


def test_the_fixtures_exercise_the_lift_and_the_example_has_fifty_models():
    assert brute_force_count(EXAMPLE) == 50
    assert vitri.prepare(LIFTED, mode="mc").summary["lift"]["count_lift_pow2"] > 0


@pytest.mark.parametrize("vtree", [None, "minfill-primal"], ids=["default-vtree", "minfill"])
@pytest.mark.parametrize("dimacs", FORMULAS.values(), ids=FORMULAS.keys())
def test_the_lifted_pysdd_count_is_the_brute_force_count(dimacs, vtree):
    settings = {} if vtree is None else {"vtree": vtree}
    assert pysdd_count.count_models(dimacs, **settings) == brute_force_count(dimacs)


@pytest.mark.parametrize("mode", ["mc", "compile"])
def test_getting_started_pysdd_command_counts_fifty(mode, vitri_cli, tmp_path):
    cnf = tmp_path / "choices.cnf"
    cnf.write_bytes(EXAMPLE)
    bundle = tmp_path / "bundle"
    subprocess.run(
        [vitri_cli, cnf, "--mode", mode, "--out-dir", bundle, "--budget-ms", "1000"],
        check=True, capture_output=True, timeout=30,
    )
    result = subprocess.run(
        ["pysdd", "-c", bundle / "reduced.cnf", "-v", bundle / "vtree.vtree",
         "-r", "0", "-R", tmp_path / "compiled.sdd", "-W", tmp_path / "compiled.vtree"],
        check=True, capture_output=True, text=True, timeout=30,
    )
    assert re.search(r"sdd model count\s*:\s*50\b", result.stdout), result.stdout
    assert (tmp_path / "compiled.sdd").stat().st_size > 0
    assert (tmp_path / "compiled.vtree").stat().st_size > 0
