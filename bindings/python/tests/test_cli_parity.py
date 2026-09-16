"""vitri.prepare returns the files `vitri --out-dir` writes for the same
settings. Runs when pytest is given --vitri-cli."""

import json
import os
import subprocess

import pytest

import vitri
from cnfs import EXAMPLE, FULLY_RESOLVED, LIFTED, REFUTED, TWO_COMPONENTS

FLAGS = {
    "mode": "--mode",
    "vtree": "--vtree",
    "budget_ms": "--budget-ms",
    "components": "--components",
    "candidates": "--candidates",
}
OFF_SWITCHES = {"simplify": "--no-simplify", "arjun": "--no-arjun"}


def command_line(settings):
    arguments = []
    for key, value in settings.items():
        if key == "dot":
            arguments += ["--dot"] if value else []
        elif key in OFF_SWITCHES:
            assert value is False, "the command line only switches stages off"
            arguments.append(OFF_SWITCHES[key])
        else:
            arguments += [FLAGS[key], str(value)]
    return arguments


def written_by_the_command_line(cli, tmp_path, dimacs, settings):
    source = tmp_path / "input.cnf"
    source.write_bytes(dimacs if isinstance(dimacs, bytes) else dimacs.encode())
    bundle = tmp_path / "bundle"
    # The command line reads VITRI_* variables into its settings; the library
    # call does not.
    environment = {k: v for k, v in os.environ.items() if not k.startswith("VITRI_")}
    subprocess.run(
        [str(cli), str(source), "-o", str(bundle), *command_line(settings)],
        env=environment,
        capture_output=True,
        check=True,
        timeout=600,
    )
    return {
        path.relative_to(bundle).as_posix(): path.read_bytes()
        for path in bundle.rglob("*")
        if path.is_file()
    }


CASES = {
    "example": (EXAMPLE, {"vtree": "minfill-primal"}),
    "lifted": (LIFTED, {"vtree": "minfill-primal", "mode": "mc"}),
    "split-with-dot": (TWO_COMPONENTS, {"vtree": "minfill-primal", "mode": "compile", "dot": True}),
    "whole-with-dot": (
        TWO_COMPONENTS,
        {"vtree": "minfill-primal", "mode": "compile", "components": "whole", "dot": True},
    ),
    "stages-off": (LIFTED, {"vtree": "minfill-primal", "simplify": False, "arjun": False}),
    "fully-resolved": (FULLY_RESOLVED, {}),
    "refuted": (REFUTED, {}),
}


@pytest.mark.parametrize(("dimacs", "settings"), CASES.values(), ids=CASES.keys())
def test_prepare_returns_the_files_the_command_line_writes(vitri_cli, tmp_path, dimacs, settings):
    result = vitri.prepare(dimacs, **settings)
    on_disk = written_by_the_command_line(vitri_cli, tmp_path, dimacs, settings)
    assert sorted(result.files) == sorted(on_disk)
    assert result.files == on_disk
    summary = result.summary
    assert sorted(summary["files"]) == sorted(on_disk)
    record = json.loads(on_disk["preprocess.json"])
    assert summary["lift"]["count_lift_pow2"] == record["count_lift_pow2"]
    assert summary["lift"]["weight_lift"] == record["weight_lift"]
