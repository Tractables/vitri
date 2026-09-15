import pathlib

import pytest


def pytest_addoption(parser):
    parser.addoption(
        "--vitri-cli",
        metavar="PATH",
        help="the vitri executable, to compare its bundles with vitri.prepare",
    )


@pytest.fixture
def vitri_cli(request):
    path = request.config.getoption("--vitri-cli")
    if path is None:
        pytest.skip("give --vitri-cli to compare with the command line")
    return pathlib.Path(path)
