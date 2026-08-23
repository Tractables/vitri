{
  description = "BackBone Extractor for CaDiCal";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    cadical = {
      url = "github:meelgroup/cadical/master";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
  outputs =
    {
      self,
      nixpkgs,
      cadical,
    }:
    let
      inherit (nixpkgs) lib;
      systems = lib.intersectLists lib.systems.flakeExposed lib.platforms.linux;
      forAllSystems = lib.genAttrs systems;
      nixpkgsFor = forAllSystems (system: nixpkgs.legacyPackages.${system});
      fs = lib.fileset;

      cadiback-package =
        {
          stdenv,
          cmake,
          git,
          cadical,
        }:
        stdenv.mkDerivation {
          name = "cadiback";
          src = fs.toSource {
            root = ./.;
            fileset = fs.unions [
              ./CMakeLists.txt
              ./config.hpp.in
              ./test
              ./include
              ./cadiback.cpp
              ./main.cpp
              ./VERSION
            ];
          };

          nativeBuildInputs = [ cmake git ];
          buildInputs = [ cadical ];

          cmakeFlags = [
            "-Dcadical_DIR=${cadical}"
          ];
        };

    in
    {
      packages = forAllSystems (
        system:
        let
          cadiback = nixpkgsFor.${system}.callPackage cadiback-package {
            cadical = cadical.packages.${system}.cadical;
          };
        in
        {
          inherit cadiback;
          default = cadiback;
        }
      );
    };
}
