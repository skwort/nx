{
  description = "nx — update awareness for NixOS";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{
      self,
      flake-parts,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "aarch64-linux"
        "x86_64-linux"
      ];

      perSystem =
        { lib, pkgs, ... }:
        {
          packages = lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux rec {
            nx = pkgs.callPackage ./nix/package.nix { };
            default = nx;
          };

          devShells.default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              clippy
              rustc
              rustfmt
            ];
          };

          formatter = pkgs.nixfmt;
        };

      flake.nixosModules = rec {
        nx = import ./nix/module.nix { inherit self; };
        default = nx;
      };
    };
}
