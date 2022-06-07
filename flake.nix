{
  inputs = {
    nixpkgs = {
      url = "github:nixos/nixpkgs/nixos-unstable";
    };
    flake-utils = {
      url = "github:numtide/flake-utils";
    };
    naersk = {
      url = "github:nix-community/naersk";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    devshell = {
      url = "github:numtide/devshell";
    };
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
  };
  outputs = { self, nixpkgs, flake-utils, naersk, devshell, ... } @ inputs:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ devshell.overlay ];
        };
        naersk-lib = naersk.lib."${system}";
      in
      rec {
        # `nix build`
        packages.satysfi-language-server = naersk-lib.buildPackage {
          pname = "satysfi-language-server";
          root = ./.;
        };
        packages.default = packages.satysfi-language-server;

        # `nix run`
        apps.satysfi-language-server = flake-utils.lib.mkApp {
          drv = packages.satysfi-language-server;
        };
        apps.default = apps.satysfi-language-server;

        # `nix develop`
        devShell = pkgs.devshell.mkShell {
          imports = [
            (pkgs.devshell.importTOML ./devshell.toml)
          ];
        };
      }
    );
}
