{ self, pkgs, lib, ... }@args:

import ./nix-build.nix {
  flakeOutput = "example-dynamic-deps";
  inputsFrom = [ pkgs.example-dynamic-deps ];
  cmdline = "main";
  expectedStdout = "Hello dynamic-deps example!";
} args
