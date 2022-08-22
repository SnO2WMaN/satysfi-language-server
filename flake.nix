{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    naersk = {
      url = "github:nix-community/naersk";
      inputs.nixpkgs.follows = "nixpkgs";
    };

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
    devshell,
    flake-utils,
    ...
  } @ inputs:
    flake-utils.lib.eachDefaultSystem (
      system: let
        inherit (pkgs) lib;
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            devshell.overlay
          ];
        };
        naersk' = pkgs.callPackage naersk {};
      in rec {
        # `nix build`
        packages.satysfi-language-server = naersk'.buildPackage {
          pname = "satysfi-language-server";
          root = builtins.path {
            path = ./.;
            filter = name: type:
              (lib.hasPrefix (toString ./src) name)
              || (name == toString ./Cargo.toml)
              || (name == toString ./Cargo.lock);
          };
        };
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
            rustfmt

            # develop
            alejandra
            taplo-cli
          ];
        };
      }
    );
}
