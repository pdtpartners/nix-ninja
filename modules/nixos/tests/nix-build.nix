# Returns a NixOS test module that strictly exercises the Nix build of an
# output of nix-ninja flake.
#
# It starts the VM with the flake inputs and inputs to the output derivation
# cached, so the Nix build can run offline and only builds the derivation and
# nothing more.

# Output name it should `nix build ${self}#${flakeOutput}`.
{ flakeOutput
# Inputs of packages it should cache in the VM /nix/store.
, inputsFrom
# Cmdline to run binary from built outPath.
, cmdline
# Expected stdout from the binary it builds.
, expectedStdout
}:

{ self, pkgs, lib, ... }:
let
  # Filtered (allowlisted) copy of the flake for the VM to build from, so the
  # test derivation only depends on files that affect what it builds —
  # changes to e.g. README or CI config don't invalidate the tests. The
  # allowlist must cover everything the in-VM `nix build` evaluates,
  # producing filesets identical to the ones the flake modules construct
  # (same contents → same store paths → the prebuilt closures cached in the
  # VM still match).
  flakeSrc = lib.fileset.toSource {
    root = ../../..;
    fileset = self.inputs.globset.lib.globs ../../.. [
      "flake.nix"
      "flake.lock"
      "modules/**"
      "Cargo.lock"
      "**/Cargo.toml"
      "**/*.rs"
      "deny.toml"
    ];
  };

  # Note: no `lib.subtractLists inputsFrom` here (unlike `pkgs.mkShell`, where
  # this pattern comes from): comparing derivations forces their `outPath`,
  # and the `inputsFrom` derivations are content-addressed, so that would
  # require the ca-derivations feature on the host evaluator. The examples
  # never appear in their own input lists anyway.
  mergeInputs = name:
    lib.flatten (lib.catAttrs name inputsFrom);

  # Extracted from `pkgs.mkShell` to capture the closure of inputs of a
  # derivation. I'd like to use `<drv>.inputDerivation` but getting an error
  # from Nix@2.30 atm:
  #
  # ```sh
  # error: derivation names are allowed to end in '.drv' only if they produce a
  # single derivation file
  # ```
  inputsClosure = pkgs.stdenv.mkDerivation {
    name = "inputs-for-${flakeOutput}";
    buildInputs = mergeInputs "buildInputs";
    nativeBuildInputs = mergeInputs "nativeBuildInputs";
    propagatedBuildInputs = mergeInputs "propagatedBuildInputs";
    propagatedNativeBuildInputs = mergeInputs "propagatedNativeBuildInputs";

    phases = [ "buildPhase" ];

    buildPhase = ''
      export >> "$out"
    '';
  };

in {
  nodes.machine = {
    virtualisation = {
      # Closures that are made available to VM, these cache all inputs & flake
      # inputs so that during the NixOS test it only needs to build the dynamic
      # derivation.
      additionalPaths = [
        inputsClosure
      ] ++ (builtins.attrValues self.inputs);
    };

    environment.systemPackages = with pkgs; [
      git
      nix-ninja
      nix-ninja-task
    ];

    nix.package = self.inputs.nix.packages.${pkgs.stdenv.hostPlatform.system}.nix;

    nix.extraOptions = ''
      experimental-features = nix-command flakes dynamic-derivations ca-derivations recursive-nix
      extra-system-features = builder-rpc-v0
    '';
  };

  testScript = ''
    start_all()

    result = machine.succeed("nix build --print-out-paths ${flakeSrc}#${flakeOutput}").strip()
    out = machine.succeed(f"{result}/${cmdline}")
    assert "${expectedStdout}" in out
  '';
}
