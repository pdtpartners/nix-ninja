{
  description = "Ninja compatible incremental C/C++ build system with Nix ";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    nix = {
      url = "github:NixOS/nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs-23-11.follows = "";
      inputs.nixpkgs-regression.follows = "";
    };
    globset = {
      url = "github:pdtpartners/globset";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-analyzer-src.follows = "";
    };
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };

    # The Cargo git dependencies, as locked inputs so their narHashes are
    # recorded. That lets evaluation resolve them from the Nix store without
    # network access (Cargo.lock alone records only the rev, which is not
    # enough) — needed by the offline NixOS test VMs, which cache all flake
    # inputs. Keep their revs in sync with Cargo.lock; the vendoring override
    # in `modules/flake/overlays.nix` checks this.
    harmonia = {
      url = "github:nix-community/harmonia";
      flake = false;
    };
    igraph = {
      url = "github:hinshun/igraph?ref=performance-improvements";
      flake = false;
    };
    n2 = {
      url = "github:hinshun/n2?ref=feature/minimal-pub";
      flake = false;
    };
  };

  outputs = inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "x86_64-linux" ];
      imports = [ ./modules ];
      flake = { inherit (inputs.nixpkgs) lib; };
    };
}
