{
  description = "a tool for reducing SAT formulas using structured bounded variable addition";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
  };
  outputs =
    {
      self,
      nixpkgs,
    }:
    let
      inherit (nixpkgs) lib;
      systems = lib.intersectLists lib.systems.flakeExposed lib.platforms.linux;
      forAllSystems = lib.genAttrs systems;
      nixpkgsFor = forAllSystems (system: nixpkgs.legacyPackages.${system});
      fs = lib.fileset;
      sbva-package =
        {
          stdenv,
          fetchFromGitHub,
          cmake,
          autoPatchelfHook,
        }:
        stdenv.mkDerivation {
          name = "sbva";
          src = fs.toSource {
            root = ./.;
            fileset = fs.unions [
              ./CMakeLists.txt
              ./cmake
              ./src
              ./sbvaConfig.cmake.in
              ./eigen-3.4.0
            ];
          };
          nativeBuildInputs = [
            cmake
            autoPatchelfHook
          ];
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          sbva = nixpkgsFor.${system}.callPackage sbva-package { };
        in
        {
          inherit sbva;
          default = sbva;
        }
      );
    };
}
