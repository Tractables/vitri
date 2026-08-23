{
  description = "Simplified Satisfiability Solver";
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

      cadical-package =
        {
          stdenv,
          cmake,
        }:
        stdenv.mkDerivation {
          name = "cadical";
          src = fs.toSource {
            root = ./.;
            fileset = fs.unions [
              ./CMakeLists.txt
              ./src
              ./test
              ./VERSION
            ];
          };
          nativeBuildInputs = [ cmake ];

          configurePhase = ''
            cmake . -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_POSITION_INDEPENDENT_CODE=ON
          '';

          buildPhase = ''
            cmake --build build -j8
          '';

          installPhase = ''
            cmake --install build --prefix $out
          '';
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          cadical = nixpkgsFor.${system}.callPackage cadical-package { };
        in
        {
          inherit cadical;
          default = cadical;
        }
      );
    };
}
