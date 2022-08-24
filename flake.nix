{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    naersk = {
      url = "github:nix-community/naersk";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nix-filter.url = "github:numtide/nix-filter";

    # dev
    devshell.url = "github:numtide/devshell";
    flake-utils.url = "github:numtide/flake-utils";
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
  };
  outputs = {
    self,
    nixpkgs,
    naersk,
    nix-filter,
    devshell,
    flake-utils,
    ...
  } @ inputs:
    {
      overlays.default = final: prev: let
        naersk' = final.callPackage naersk {};
      in {
        satysfi-language-server = naersk'.buildPackage {
          pname = "satysfi-language-server";
          root = with nix-filter.lib;
            filter {
              root = ./.;
              include = [
                "Cargo.toml"
                "Cargo.lock"
                (inDirectory "src")
              ];
            };
        };
      };
    }
    // flake-utils.lib.eachDefaultSystem (
      system: let
        inherit (pkgs) lib;
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            devshell.overlay
            self.overlays.default
          ];
        };
      in rec {
        # `nix build`
        packages.satysfi-language-server = pkgs.satysfi-language-server;
        packages.default = packages.satysfi-language-server;

        # `nix run`
        apps.satysfi-language-server = flake-utils.lib.mkApp {
          drv = packages.satysfi-language-server;
        };
        apps.default = apps.satysfi-language-server;

        devShell = pkgs.devshell.mkShell {
          commands = with pkgs; [
            {
              package = "treefmt";
              category = "formatter";
            }
          ];
          packages = with pkgs; [
            gcc
            cargo
            rustc

            # develop
            alejandra
            taplo-cli
            rustfmt
          ];
        };
      }
    );
}
