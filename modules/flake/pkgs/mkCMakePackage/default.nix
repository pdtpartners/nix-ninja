{ cmake
, coreutils
, nix
, nix-ninja
, nix-ninja-task
, patchelf
, stdenv
}:

{ name ? "${args'.pname}-${args'.version}"
, src
, target
, cmakeFlags ? [ ]
, nativeBuildInputs ? [ ]
, ...
}@args':

let
  normalizedTarget = builtins.replaceStrings [ "/" ] [ "-" ] target;

  ninjaDrv = stdenv.mkDerivation (args' // {
    name = "${name}.drv";

    nativeBuildInputs = [
      cmake
      coreutils
      nix
      nix-ninja
      nix-ninja-task
      patchelf
    ] ++ nativeBuildInputs;

    requiredSystemFeatures = [ "recursive-nix" ];

    cmakeFlags = [
      "-G"
      "Ninja"
      "-DCMAKE_MAKE_PROGRAM=${nix-ninja}/bin/nix-ninja"
      "-DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY"
      "-DCMAKE_C_COMPILER_FORCED=ON"
      "-DCMAKE_CXX_COMPILER_FORCED=ON"
      "-DCMAKE_C_COMPILER_WORKS=ON"
      "-DCMAKE_CXX_COMPILER_WORKS=ON"
    ] ++ cmakeFlags;

    preConfigure = ''
      export NIX_NINJA_DRV="true"
      export NINJA="${nix-ninja}/bin/nix-ninja"
      export NIX_CONFIG="extra-experimental-features = nix-command ca-derivations dynamic-derivations"
    '';

    buildPhase = ''
      runHook preBuild
      nix-ninja ${target}
      runHook postBuild
    '';

    dontUseCmakeInstall = true;
    dontUseCmakeCheck = true;

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

in
ninjaDrv
