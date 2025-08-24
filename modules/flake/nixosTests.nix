{ self, lib, flake-parts-lib, ... }:
let
  inherit (lib)
    mkOption
    types
  ;

  inherit (flake-parts-lib)
    mkPerSystemOption
  ;

in {
  options.perSystem = mkPerSystemOption {
    _file = ./nixosTests.nix;

    options.nixosTests = mkOption {
      type = types.attrsOf types.deferredModule;
      default = { };
    };
  };

  config.perSystem = { config, pkgs, ... }:
    let
      evalTest = name: module:
        (lib.nixos.evalTest {
          imports = [
            {
              inherit name;
              _module.args = { inherit self; };
            }
            module
          ];
          hostPkgs = pkgs;
          node = { inherit pkgs; };
        }).config.result;

      testRigs = lib.mapAttrs (name: module: evalTest name module) config.nixosTests;

    in {
      /* For each nixosTest, add an `apps` target that allows the use of
         `machine.shell_interact()` for developing tests.
        
         ```sh
         nix run .#test-<name> -L
         ```
      */
      apps =
        lib.mapAttrs'
          (name: testRig:
            lib.nameValuePair
              ("test-" + name)
              {
                type = "app";
                program = "${testRig.driver}/bin/nixos-test-driver";
              }
          )
          testRigs;


      packages =
        lib.mapAttrs'
          (name: testRig: lib.nameValuePair ("driver-" + name) testRig.driver)
          testRigs;
    };
}
