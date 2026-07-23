{ self, inputs, lib, ... }:
{
  flake.overlays.internal = self: super:
    let
      craneLib = inputs.crane.mkLib self;

      src = lib.fileset.toSource {
        root = ../../.;
        fileset = inputs.globset.lib.globs ../../. [
          "Cargo.lock"
          "**/Cargo.toml"
          "**/*.rs"
        ];
      };

      # The Cargo git dependencies, mapped from a URL substring to the flake
      # input holding their checkout (see the inputs' comment in flake.nix).
      cargoGitDeps = {
        "github.com/nix-community/harmonia" = inputs.harmonia;
        "github.com/hinshun/igraph" = inputs.igraph;
        "github.com/hinshun/n2" = inputs.n2;
      };

      # Vendor the Cargo git dependencies from their locked flake inputs
      # instead of letting crane fetch them with `builtins.fetchGit`, which
      # needs network access at evaluation time.
      #
      # TODO: An alternative that would keep Cargo.lock as the single source
      # of truth (no per-dep flake input, no rev-sync check): vendor the old
      # way on the outside, ship the built vendor dir into the offline NixOS
      # test VMs via `additionalPaths`, and have the inside evaluation take it
      # through an overridable flake input (`nix build --override-input
      # cargo-vendor-dir path:<dir> ...`). The catch is that with
      # input-addressed derivations, the inside evaluation (vendor dir passed
      # in) and the outside evaluation (vendor dir computed) produce different
      # drv hashes for nix-ninja and everything downstream, so the VM would
      # rebuild the whole workspace instead of reusing the cached binaries.
      # This works out only if the derivation producing the vendor dir
      # content-addresses its output: then the output path depends solely on
      # the vendored contents, both evaluations realise the same store path,
      # and the downstream drvs coincide again.
      cargoVendorDir = craneLib.vendorCargoDeps {
        inherit src;
        overrideVendorGitCheckout = ps: drv:
          let
            p = lib.head ps;
            # `source` looks like "git+https://…?branch=…#<rev>".
            lockedRev = lib.last (lib.splitString "#" p.source);
            matches = lib.filterAttrs (infix: _: lib.hasInfix infix p.source) cargoGitDeps;
            input = lib.head (lib.attrValues matches);
          in
          lib.throwIf (matches == { }) ''
            Cargo git dependency ${p.source} has no flake input in
            `cargoGitDeps` (modules/flake/overlays.nix); add one so it can be
            vendored without network access.
          ''
            (lib.throwIfNot (lockedRev == input.rev) ''
              Cargo.lock pins ${p.name} at ${lockedRev}
              but its flake input is at ${input.rev}.
              Re-sync them with `nix flake update` and/or `cargo update`.
            ''
              (drv.overrideAttrs (_: { src = input; })));
      };

      # Common arguments can be set here to avoid repeating them later
      commonArgs = {
        inherit src cargoVendorDir;
        inherit (craneLib.crateNameFromCargoToml { inherit src; }) version;
        strictDeps = true;
        nativeBuildInputs = [
          self.pkg-config
        ];
      };

      # Build *just* the cargo dependencies, so we can reuse
      # all of that work (e.g. via cachix) when running in CI
      cargoArtifacts = craneLib.buildDepsOnly commonArgs;

      craneLibLLvmTools = craneLib.overrideToolchain
        (inputs.fenix.packages.${self.system}.complete.withComponents [
          "cargo"
          "llvm-tools"
          "rustc"
        ]);

    in {
      inherit craneLib;

      # Internal attr for code-reuse across flake modules.
      _nix-ninja = {
        inherit cargoArtifacts commonArgs src;
      };

      mkMesonPackage = self.callPackage ./pkgs/mkMesonPackage {
        inherit (self) nix-ninja nix-ninja-task;
        nix = inputs.nix.packages.${self.system}.nix;
      };

      # meson --internal symbolextractor depends on readelf.
      # meson = super.meson.overrideAttrs(o: {
      #   buildInputs = (o.buildInputs or []) ++ [
      #     self.binutils
      #   ];
      # });

      nix-ninja-llvm-coverage = craneLibLLvmTools.cargoLlvmCov (commonArgs // {
        inherit cargoArtifacts;
      });

      # Build the actual crate itself, reusing the dependency
      # artifacts from above.
      nix-ninja = craneLib.buildPackage (commonArgs // {
        inherit cargoArtifacts;
        pname = "nix-ninja";
        cargoExtraArgs = "-p nix-ninja";
      });

      nix-ninja-task = craneLib.buildPackage (commonArgs // {
        inherit cargoArtifacts;
        pname = "nix-ninja-task";
        cargoExtraArgs = "-p nix-ninja-task";
        src = lib.fileset.toSource {
          root = ../../.;
          fileset = inputs.globset.lib.globs ../../. [
            "Cargo.{toml,lock}"
            "crates/nix-{libstore,ninja-task}/Cargo.toml"
            "crates/nix-{libstore,ninja-task}/**/*.rs"
            "crates/deps-infer/Cargo.toml"
            "crates/deps-infer/**/*.rs"
          ];
        };
      });

      example-hello = self.mkMesonPackage {
        name = "example-hello";
        src = ./examples/hello;
        target = "hello";
      };

      example-header = self.mkMesonPackage {
        name = "example-header";
        src = ./examples/header;
        target = "hello";
        nativeBuildInputs = [ self.nlohmann_json ];
      };

      example-multi-source = self.mkMesonPackage {
        name = "example-multi-source";
        src = ./examples/multi-source;
        target = "main";
      };

      example-shared-lib = self.mkMesonPackage {
        name = "example-shared-lib";
        src = ./examples/shared-lib;
        target = "main";
      };

      example-dynamic-deps= self.mkMesonPackage {
        name = "example-dynamic-deps";
        src = ./examples/dynamic-deps;
        target = "main";
        nativeBuildInputs = [ self.nlohmann_json self.pkg-config ];
      };

      example-nix = self.callPackage ./examples/nix { src = inputs.nix; };
    };

  perSystem = { system, ... }: {
    _module.args.pkgs = import inputs.nixpkgs {
      inherit system;
      overlays = [ self.overlays.internal ];
    };
  };
}
