{ inputs
, aws-sdk-cpp
, boehmgc
, boost
, brotli
, busybox-sandbox-shell
, bzip2
, cmake
, curl
, doxygen
, editline
, flex
, git
, gtest
, jq
, lib
, libarchive
, libcpuid
, libsodium
, lowdown
, lowdown-unsandboxed
, lsof
, mdbook
, mdbook-linkcheck
, mercurial
, mkMesonPackage
, nlohmann_json
, openssl
, pegtl
, pkg-config
, python3
, rapidcheck
, sqlite
, stdenv
, toml11
, util-linuxMinimal
, xz
, enableDocumentation ? stdenv.hostPlatform == stdenv.buildPlatform
, enableStatic ? stdenv.hostPlatform.isStatic
, withAWS ? !enableStatic && (stdenv.hostPlatform.isLinux || stdenv.hostPlatform.isDarwin)
, withLibseccomp ? lib.meta.availableOn stdenv.hostPlatform libseccomp
, libseccomp
}:

let
  aws-sdk-cpp-nix =
    if aws-sdk-cpp == null then
      null
    else
      aws-sdk-cpp.override {
        apis = [
          "s3"
          "transfer"
        ];
        customMemoryManagement = false;
      };

  boehmgc-nix = boehmgc.override { enableLargeConfig = true; };

in mkMesonPackage {
  name = "example-lix";
  src = inputs.lix;
  patches = [
    ./disable-meson-clang-tidy.patch
  ];

  target = "src/nix/nix";

  # nixNinjaExtraInputs = [];

  dontInstall = true;
  dontFixup = true;

  nativeBuildInputs = [
    pkg-config
    flex
    jq
    cmake
    python3

    # Tests
    git
    mercurial
    jq
    lsof
  ]
  ++ lib.optionals enableDocumentation [
    (lib.getBin lowdown-unsandboxed)
    mdbook
    mdbook-linkcheck
    doxygen
  ]
  ++ lib.optionals stdenv.hostPlatform.isLinux [ util-linuxMinimal ];

  buildInputs =
  [
    boost
    brotli
    bzip2
    curl
    editline
    libsodium
    openssl
    sqlite
    xz
    gtest
    libarchive
    lowdown
    rapidcheck
    toml11
    pegtl
  ]
  ++ lib.optionals (stdenv.hostPlatform.isx86_64) [ libcpuid ]
  ++ lib.optionals withLibseccomp [ libseccomp ]
  ++ lib.optionals withAWS [ aws-sdk-cpp-nix ];

  propagatedBuildInputs = [
    boehmgc-nix
    nlohmann_json
  ];

  postPatch = ''
    patchShebangs --build tests doc/manual
  '';

  preConfigure =
  # Copy libboost_context so we don't get all of Boost in our closure.
  # https://github.com/NixOS/nixpkgs/issues/45462
  lib.optionalString (!enableStatic) ''
    mkdir -p $out/lib
    cp -pd ${boost}/lib/{libboost_context*,libboost_thread*,libboost_system*} $out/lib
    rm -f $out/lib/*.a
    ${lib.optionalString stdenv.hostPlatform.isLinux ''
      chmod u+w $out/lib/*.so.*
      patchelf --set-rpath $out/lib:${lib.getLib stdenv.cc.cc}/lib $out/lib/libboost_thread.so.*
    ''}
    ${lib.optionalString stdenv.hostPlatform.isDarwin ''
      for LIB in $out/lib/*.dylib; do
        chmod u+w $LIB
        install_name_tool -id $LIB $LIB
        install_name_tool -delete_rpath ${boost}/lib/ $LIB || true
      done
      install_name_tool -change ${boost}/lib/libboost_system.dylib $out/lib/libboost_system.dylib $out/lib/libboost_thread.dylib
    ''}
  '';

  # -O3 seems to anger a gcc bug and provide no performance benefit.
  # https://gcc.gnu.org/bugzilla/show_bug.cgi?id=114360
  # We use -O2 upstream https://gerrit.lix.systems/c/lix/+/554
  mesonBuildType = "debugoptimized";

  mesonFlags =
  [
    # Enable LTO, since it improves eval performance a fair amount
    # LTO is disabled on static due to strange linking errors
    (lib.mesonBool "b_lto" (!stdenv.hostPlatform.isStatic && stdenv.cc.isGNU))
    (lib.mesonEnable "gc" true)
    (lib.mesonBool "enable-tests" true)
    (lib.mesonBool "enable-docs" enableDocumentation)
    (lib.mesonEnable "internal-api-docs" enableDocumentation)
    (lib.mesonBool "enable-embedded-sandbox-shell" (
      stdenv.hostPlatform.isLinux && stdenv.hostPlatform.isStatic
    ))
    (lib.mesonEnable "seccomp-sandboxing" withLibseccomp)
  ]
  ++ lib.optionals stdenv.hostPlatform.isLinux [
    (lib.mesonOption "sandbox-shell" "${busybox-sandbox-shell}/bin/busybox")
  ];
}
