{ inputs
, aws-sdk-cpp
, bison
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
, gtest
, lib
, libarchive
, libblake3
, libcpuid
, libgit2
, libseccomp
, libsodium
, lowdown
, mkMesonPackage
, nlohmann_json
, openssl
, perl
, perlPackages
, pkg-config
, rapidcheck
, readline
, sqlite
, toml11
}:

mkMesonPackage {
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

  nativeBuildInputs = [
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

  buildInputs = [
    rapidcheck
    gtest
  ];

  # dontAddPrefix = true;

  mesonFlags = [
    "--prefix=/build/tmp"
    "--bindir=/build/tmp/bin"
    "--mandir=/build/tmp/man"
    (lib.mesonOption "perl:dbi_path" "${perlPackages.DBI}/${perl.libPrefix}")
    (lib.mesonOption "perl:dbd_sqlite_path" "${perlPackages.DBDSQLite}/${perl.libPrefix}")
  ];

  env = {
    # Needed for Meson to find Boost.
    # https://github.com/NixOS/nixpkgs/issues/86131.
    BOOST_INCLUDEDIR = "${lib.getDev boost}/include";
    BOOST_LIBRARYDIR = "${lib.getLib boost}/lib";
  };
}
