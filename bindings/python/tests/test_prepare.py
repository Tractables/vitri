"""The Python surface of vitri: settings, results, files and errors."""

import inspect
import os
import subprocess
import sys

import pytest

import vitri
from cnfs import EXAMPLE, FULLY_RESOLVED, MALFORMED, REFUTED, TWO_COMPONENTS

ERROR_KINDS = {
    "ConfigError",
    "SpecError",
    "EnvError",
    "InputError",
    "MismatchError",
    "ConstructionError",
    "IoError",
}


def test_the_module_exports_prepare_its_result_and_one_error_class_per_kind():
    assert set(vitri.__all__) == ERROR_KINDS | {
        "VitriError",
        "Result",
        "__version__",
        "capabilities",
        "prepare",
    }
    assert issubclass(vitri.VitriError, Exception)
    for name in ERROR_KINDS:
        assert issubclass(getattr(vitri, name), vitri.VitriError)


def test_the_package_version_is_the_library_version():
    assert vitri.__version__ == vitri.capabilities()["vitri_version"]


def test_the_keywords_of_prepare_are_the_request_keys():
    parameters = inspect.signature(vitri.prepare).parameters.values()
    keywords = {p.name for p in parameters if p.kind is inspect.Parameter.KEYWORD_ONLY}
    assert keywords == set(vitri.capabilities()["request_keys"]) - {"format"}


def test_a_built_run_returns_its_summary_and_every_file():
    result = vitri.prepare(TWO_COMPONENTS, mode="compile", vtree="minfill-primal")
    summary = result.summary
    files = result.files
    assert result.status == summary["status"] == "built"
    assert summary["format"] == vitri.capabilities()["result_format"]
    assert list(files) == summary["files"]
    assert result.reduced_cnf == files["reduced.cnf"].decode()
    assert result.vtree == files["vtree.vtree"].decode()
    assert summary["vtree"]["leaves"] == summary["reduced"]["variables"]
    assert summary["request"]["vtree"] == "minfill-primal"
    assert any(path.startswith("components/") for path in files)


def test_text_and_bytes_give_the_same_bundle():
    text = vitri.prepare(EXAMPLE.decode(), vtree="minfill-primal")
    raw = vitri.prepare(EXAMPLE, vtree="minfill-primal")
    assert text.files == raw.files


def test_settings_left_out_take_the_library_defaults():
    request = vitri.prepare(EXAMPLE).summary["request"]
    assert request["vtree"] == vitri.capabilities()["default_vtree"]
    assert request["budget_ms"] is None
    assert request["dot"] is False


def test_every_component_policy_the_capabilities_list_is_accepted():
    for policy in vitri.capabilities()["components"]:
        result = vitri.prepare(
            TWO_COMPONENTS, mode="compile", vtree="minfill-primal", components=policy
        )
        assert result.summary["request"]["components"] == policy


def test_dot_adds_a_graphviz_file_for_every_vtree():
    plain = vitri.prepare(TWO_COMPONENTS, mode="compile", vtree="minfill-primal").files
    drawn = vitri.prepare(TWO_COMPONENTS, mode="compile", vtree="minfill-primal", dot=True).files
    vtrees = [path for path in plain if path.endswith(".vtree")]
    dots = [path for path in drawn if path.endswith(".dot")]
    assert len(dots) == len(vtrees) > 0
    assert set(drawn) - set(dots) == set(plain)


@pytest.mark.parametrize(
    ("dimacs", "status"),
    [(REFUTED, "refuted"), (FULLY_RESOLVED, "fully_resolved")],
    ids=["refuted", "fully_resolved"],
)
def test_a_formula_preprocessing_settles_has_no_vtree(dimacs, status):
    result = vitri.prepare(dimacs)
    assert result.status == status
    assert result.vtree is None
    assert result.summary["vtree"] is None
    assert list(result.files) == ["reduced.cnf", "preprocess.json"]


def test_write_puts_every_file_under_the_directory(tmp_path):
    result = vitri.prepare(TWO_COMPONENTS, mode="compile", vtree="minfill-primal", dot=True)
    target = tmp_path / "new" / "bundle"
    result.write(target)
    written = {
        path.relative_to(target).as_posix(): path.read_bytes()
        for path in target.rglob("*")
        if path.is_file()
    }
    assert written == result.files


def test_write_over_a_file_raises_io_error_naming_the_path(tmp_path):
    blocker = tmp_path / "occupied"
    blocker.write_text("")
    result = vitri.prepare(EXAMPLE, vtree="minfill-primal")
    with pytest.raises(vitri.IoError, match="occupied"):
        result.write(blocker)


@pytest.mark.parametrize(
    ("settings", "error", "named"),
    [
        ({"mode": "count"}, vitri.ConfigError, "count"),
        ({"components": "both"}, vitri.ConfigError, "both"),
        ({"candidates": 0}, vitri.ConfigError, "candidates"),
        ({"vtree": "no-such-construction"}, vitri.SpecError, "no-such-construction"),
    ],
    ids=["mode", "components", "candidates", "vtree"],
)
def test_a_refused_setting_raises_its_error_kind_naming_the_value(settings, error, named):
    with pytest.raises(error) as raised:
        vitri.prepare(EXAMPLE, **settings)
    assert named in str(raised.value)


def test_dimacs_that_does_not_parse_raises_input_error():
    with pytest.raises(vitri.InputError):
        vitri.prepare(MALFORMED)


@pytest.mark.parametrize(
    ("args", "kwargs"),
    [
        ((EXAMPLE,), {"seed": 1}),
        ((EXAMPLE, "mc"), {}),
        ((123,), {}),
        ((EXAMPLE,), {"simplify": "yes"}),
        ((EXAMPLE,), {"budget_ms": "1000"}),
    ],
    ids=["unknown-keyword", "positional-setting", "dimacs-int", "simplify-str", "budget-str"],
)
def test_an_unknown_keyword_or_a_wrong_type_raises_type_error(args, kwargs):
    with pytest.raises(TypeError):
        vitri.prepare(*args, **kwargs)


def test_a_bad_value_in_a_variable_the_vendored_stack_reads_raises_env_error():
    script = (
        "import sys, vitri\n"
        "try:\n"
        "    vitri.prepare(sys.stdin.buffer.read())\n"
        "except vitri.EnvError as error:\n"
        "    print(error)\n"
        "    sys.exit(3)\n"
    )
    done = subprocess.run(
        [sys.executable, "-c", script],
        input=EXAMPLE,
        env={**os.environ, "VITRI_ARJUN_NO_BVE": "0"},
        capture_output=True,
        timeout=600,
    )
    assert done.returncode == 3, done.stderr.decode()
    assert b"VITRI_ARJUN_NO_BVE" in done.stdout


def test_a_result_is_not_constructed_from_python():
    with pytest.raises(TypeError):
        vitri.Result()
