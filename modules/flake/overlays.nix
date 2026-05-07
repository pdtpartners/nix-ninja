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

      # Common arguments can be set here to avoid repeating them later
      commonArgs = {
        inherit src;
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
            "crates/nix-{libstore,ninja-task,builder-rpc-client}/Cargo.toml"
            "crates/nix-{libstore,ninja-task,builder-rpc-client}/**/*.rs"
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
