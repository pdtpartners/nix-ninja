{ lib
, coreutils
, meson
, nix
, nix-ninja
, nix-ninja-task
, patchelf
, stdenv
}:

{ name ? "${args'.pname}-${args'.version}"
, src
, target
, nativeBuildInputs ? [ ]
, nixNinjaExtraInputs ? [ ]
, ...
}@args':

let
  normalizedTarget = builtins.replaceStrings ["/"] ["-"] target;

  extraInputs = builtins.concatStringsSep "," nixNinjaExtraInputs;

  ninjaDrv = stdenv.mkDerivation (args' // {
    name = "${name}.drv";

    nativeBuildInputs = [
      coreutils
      meson
      nix
      nix-ninja
      nix-ninja-task
      patchelf
    ] ++ nativeBuildInputs;

    requiredSystemFeatures = [ "recursive-nix" ];

    preConfigure = ''
      export NIX_NINJA_DRV="true"
      export NINJA="${nix-ninja}/bin/nix-ninja"
      export NIX_CONFIG="extra-experimental-features = nix-command ca-derivations dynamic-derivations"
    '' + (lib.optionalString (builtins.length nixNinjaExtraInputs > 0) ''
      export NIX_NINJA_EXTRA_INPUTS="${extraInputs}"
    '');

    buildPhase = ''
      runHook preBuild
      nix-ninja ${target}
      runHook postBuild
    '';

    dontUseMesonInstall = true;
    dontUseMesonCheck = true;

    # stdenv adds a -rpath with a self reference but self references are not
    # allowed by text output.
    NIX_NO_SELF_RPATH = true;

    __contentAddressed = true;
    outputHashMode = "text";
    outputHashAlgo = "sha256";

    passthru = {
      target = builtins.outputOf ninjaDrv.outPath normalizedTarget;
    };
  });

in ninjaDrv
