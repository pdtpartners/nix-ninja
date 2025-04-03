{
  description = "Ninja compatible incremental C/C++ build system with Nix ";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    nix.url = "github:hinshun/nix/2.27.1-fix-nix-missing-includes";

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

    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
  };

  outputs = inputs:
    let
      system = "x86_64-linux";

      pkgs = import inputs.nixpkgs {
        inherit system;
      };

      inherit (pkgs) lib;

      craneLib = inputs.crane.mkLib pkgs;

      src = lib.fileset.toSource {
        root = ./.;
        fileset = inputs.globset.lib.globs ./. [
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
        nativeBuildInputs = with pkgs; [
          pkg-config
        ];
      };

      craneLibLLvmTools = craneLib.overrideToolchain
        (inputs.fenix.packages.${system}.complete.withComponents [
          "cargo"
          "llvm-tools"
          "rustc"
        ]);

      # Build *just* the cargo dependencies, so we can reuse
      # all of that work (e.g. via cachix) when running in CI
      cargoArtifacts = craneLib.buildDepsOnly commonArgs;

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
          root = ./.;
          fileset = inputs.globset.lib.globs ./. [
            "Cargo.toml"
            "Cargo.lock"
            "crates/nix-libstore/Cargo.toml"
            "crates/nix-libstore/**/*.rs"
            "crates/nix-ninja-task/Cargo.toml"
            "crates/nix-ninja-task/**/*.rs"
          ];
        };
      });

      default = pkgs.buildEnv {
        name = "nix-ninja";
        paths = [
          nix-ninja
          nix-ninja-task
        ];
      };

      mkMesonPackage = pkgs.callPackage ./mkMesonPackage.nix {
        inherit nix-ninja nix-ninja-task;
      };

      example-hello = mkMesonPackage {
        name = "example-hello";
        src = ./examples/hello;
        target = "hello";
      };

      example-header = mkMesonPackage {
        name = "example-header";
        src = ./examples/header;
        target = "hello";
      };

      example-incremental = mkMesonPackage {
        name = "example-header";
        src = ./examples/incremental;
        target = "main";
      };

      example-nix = mkMesonPackage {
        name = "example-nix";
        src = inputs.nix;
        target = "src/nix/nix";

        nixNinjaExtraInputs = [
          "src/libexpr/libnixexpr.so.p/meson-generated_.._parser-tab.cc.o:../src/libexpr/parser.y"
          "src/libexpr/libnixexpr.so.p/meson-generated_.._lexer-tab.cc.o:../src/libexpr/parser.y"
          "src/libexpr/libnixexpr.so.p/meson-generated_.._lexer-tab.cc.o:../src/libexpr/lexer.l"
          "src/libexpr/libnixexpr.so.p/eval.cc.o:../src/libexpr/parser.y"
          "src/libexpr/libnixexpr.so.p/lexer-helpers.cc.o:../src/libexpr/parser.y"
        ];

        nativeBuildInputs = with pkgs; [
          aws-sdk-cpp
          bison
          boehmgc
          boost
          brotli
          busybox-sandbox-shell
          bzip2
          cmake
          curl
          doxygen
          editline
          flex
          libarchive
          libblake3
          libcpuid
          libgit2
          libseccomp
          libsodium
          lowdown
          nlohmann_json
          openssl
          perl
          pkg-config
          readline
          sqlite
          toml11
        ];

        buildInputs = with pkgs; [
          rapidcheck
          gtest
        ];

        # dontAddPrefix = true;

        mesonFlags = with pkgs; [
          "--prefix=/build/tmp"
          "--bindir=/build/tmp/bin"
          "--mandir=/build/tmp/man"
          (lib.mesonOption "perl:dbi_path" "${perlPackages.DBI}/${perl.libPrefix}")
          (lib.mesonOption "perl:dbd_sqlite_path" "${perlPackages.DBDSQLite}/${perl.libPrefix}")
        ];

        env = with pkgs; {
          # Needed for Meson to find Boost.
          # https://github.com/NixOS/nixpkgs/issues/86131.
          BOOST_INCLUDEDIR = "${lib.getDev boost}/include";
          BOOST_LIBRARYDIR = "${lib.getLib boost}/lib";
        };
      };

    in
    {
      inherit pkgs;

      checks.${system} = {
        # Build the crate as part of `nix flake check` for convenience
        inherit nix-ninja;

        # Run clippy (and deny all warnings) on the crate source,
        # again, reusing the dependency artifacts from above.
        #
        # Note that this is done as a separate derivation so that
        # we can block the CI if there are issues here, but not
        # prevent downstream consumers from building our crate by itself.
        nix-ninja-clippy = craneLib.cargoClippy (commonArgs // {
          inherit cargoArtifacts;
          cargoClippyExtraArgs = "--all-targets -- --deny warnings";
        });

        nix-ninja-doc = craneLib.cargoDoc (commonArgs // {
          inherit cargoArtifacts;
        });

        # Check formatting
        nix-ninja-fmt = craneLib.cargoFmt {
          inherit src;
        };

        nix-ninja-toml-fmt = craneLib.taploFmt {
          src = lib.fileset.toSource {
            root = ./.;
            fileset = inputs.globset.lib.globs ./. [
              "**/*.toml"
            ];
          };

          # taplo arguments can be further customized below as needed
          # taploExtraArgs = "--config ./taplo.toml";
        };

        # Audit dependencies
        nix-ninja-audit = craneLib.cargoAudit {
          inherit src;
          inherit (inputs) advisory-db;
        };

        # Audit licenses
        nix-ninja-deny = craneLib.cargoDeny {
          inherit src;
        };

        # Run tests with cargo-nextest
        # Consider setting `doCheck = false` on `nix-ninja` if you do not want
        # the tests to run twice
        nix-ninja-nextest = craneLib.cargoNextest (commonArgs // {
          inherit cargoArtifacts;
          partitions = 1;
          partitionType = "count";
          cargoNextestPartitionsExtraArgs = "--no-tests=pass";
        });
      };

      packages.${system} = {
        inherit default nix-ninja nix-ninja-task;

        inherit (pkgs) nix;

        nix-ninja-llvm-coverage = craneLibLLvmTools.cargoLlvmCov (commonArgs // {
          inherit cargoArtifacts;
        });

        inherit
          example-hello
          example-header
          example-incremental
          example-nix
        ;
      };

      devShells.${system}.default = craneLib.devShell {
        checks = inputs.self.checks.${system};

        packages = with pkgs; [
          gnumake
          just
          meson
          agg
        ];
      };
    };
}
