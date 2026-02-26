{ self, pkgs, lib, ... }@args:

import ./nix-build.nix {
  flakeOutput = "example-cmake";
  inputsFrom = [ pkgs.example-cmake ];
  cmdline = "hello";
  expectedStdout = "Hello CMake example!";
} args
