# Introduction to Dynamic derivations

Currently, Nix caches successful builds at the package level. Unfortunately, any
small change to the source invalidates this cache---if a single source file or
dependency is modified, then the entire package needs to be rebuilt from
scratch.

[Dynamic derivations](https://github.com/NixOS/rfcs/blob/master/rfcs/0092-plan-dynamism.md)
are an experimental Nix feature that enables users to specify a more granular
build graph by dynamically generating derivations at build-time. The goal is for
modifications to the source code to only invalidate _some_ of the build cache,
and re-use work from previous builds. This gives us incremental compilation and
[better integrations between Nix and other languages (`$lang2nix`)](https://wiki.nixos.org/wiki/Nix_For_Lang_Packaging)
_without_ using
[import from derivation (IFD)](https://nix.dev/manual/nix/2.33/language/import-from-derivation).

It's been in the works for a while, and finally at the point where we can start
experimenting.

> This document distills the docs and source code into a condensed write-up to
> help you build tooling that leverages dynamic derivations.

## Feature flags

In order to leverage incremental compilation in Nix, we need to enable these
experimental features:

- [`dynamic-derivations`](https://github.com/NixOS/rfcs/blob/master/rfcs/0092-plan-dynamism.md):
  Allows derivations to generate other derivations at build time.
- [`ca-derivations`](https://github.com/NixOS/rfcs/blob/master/rfcs/0062-content-addressed-paths.md):
  Ensures identical build outputs always get put into the same Nix store path,
  regardless of inputs. This enables the "early cutoff optimization" described
  in
  [Build Systems a la Carte](https://www.microsoft.com/en-us/research/wp-content/uploads/2018/03/build-systems.pdf).

At a high-level, this is all made possible because two new features:

- `text`
  [output hash mode](https://nix.dev/manual/nix/2.33/language/advanced-attributes.html?highlight=outputhash#adv-attr-outputHashMode):
  Allows derivations to write an ATerm-serialized derivation into `$out`.
- `nix-computed-output` "placeholder": Allows Nix to fill in derivation output
  hashes at build-time.

We'll get to them later, but first there's some necessary context to go through.

## Content-addressed derivations

A derivation can be one of
[three types](https://book.divnix.com/ch04-00-derivations.html#types-of-derivations):

1. Input-addressed: These are the default in Nix. Changing any of the inputs to
   the derivation causes the output store path to be recomputed.
2. Fixed-output derivations (FODs): These do not refer to any other derivations,
   and are defined by their content. They contain some hash to ensure that the
   built artifact is reproducible.
3. Floating content-addressed derivations (CA derivations): Requires the
   experimental `ca-derivations` feature. The output store path is computed from
   its output.

One benefit of (3) over (1) is that changes that do not affect the output build
artifact (e.g. adding/removing comments) will _not_ change where the output
artifact is stored. This enables the "early cutoff optimization".

A derivation is a CA derivation if its attribute set has:

- `__contentAddressed = true`
- `outputHashAlgo` and `outputHashMode` are set
- `outputHash` is _not_ set

Notably, FODs have content hashes in `outputHash` but are _not_
content-addressed because the "address" part refers to how the derivation is
referenced, i.e. FODs are referenced by input-addressed Nix store paths.

> Side thought: If FODs were wrapped by CA derivations, then they could be
> cached across Nix stores with different Nix store prefixes like `/opt/store`
> and also survive unnecessary rebuild-the-worlds when inputs like `curl`
> change.

Moving on,
[`outputHashMode`](https://nix.dev/manual/nix/2.33/language/advanced-attributes.html?highlight=outputhash#adv-attr-outputHashMode)
has four possible values:

- `flat` (default) for a single non-executable file
- `recursive` or `nar` for directories
- `text` used for `dynamic-derivations` experimental feature
- `git` used for `git-hashing` experimental feature

FODs typically use the `flat` mode, which doesn't allow references to other Nix
store objects. However, the new `text` mode allows references (but not
self-references). The hash mode `text` is used for `.drv` outputs which Nix will
continue building directly without IFD.

## Derivation file formats

Derivation files (`*.drv`) are stored in the Nix store using the ATerm format.
Nix also accepts derivations written using JSON, and can serialize them into
ATerm for you. I would recommend using JSON derivation format for simplicity.

### ATerm derivation format

[ATerm](https://homepages.cwi.nl/~daybuild/daily-books/technology/aterm-guide/aterm-guide.html)
is an existing data format for representing tree-like structures (similar to XML
or JSON). Derivation files (`*.drv`) are
[stored in the Nix store using the ATerm format](https://nix.dev/manual/nix/2.33/protocols/derivation-aterm).

From reading `NixOS/nix` source code, it's considered cleaner to use
`DrvWithVersion` when leveraging `dynamic-derivations` features, but it doesn't
seem strictly necessary. Its only purpose is to check whether the current Nix
daemon is compatible with the `DrvWithVersion(...)` on disk. They may be
incompatible if you previously had `dynamic-derivations` enabled but disabled
it, or copied over a derivation from a Nix store that had `dynamic-derivations`
enabled when you do not have it enabled.

<details>
<summary>Example ATerm-serialized Derivation</summary>

```sh
$ cat /nix/store/0m4y3j4pnivlhhpr5yqdvlly86p93fwc-busybox.drv
Derive([("out","/nix/store/p9wzypb84a60ymqnhqza17ws0dvlyprg-busybox","r:sha256","42b4c49d04c133563fa95f6876af22ad9910483f6e38c6ecd90e4d802bca08d4")],[],[],"builtin","builtin:fetchurl",[],[("builder","builtin:fetchurl"),("executable","1"),("impureEnvVars","http_proxy https_proxy ftp_proxy all_proxy no_proxy"),("name","busybox"),("out","/nix/store/p9wzypb84a60ymqnhqza17ws0dvlyprg-busybox"),("outputHash","sha256-QrTEnQTBM1Y/qV9odq8irZkQSD9uOMbs2Q5NgCvKCNQ="),("outputHashAlgo",""),("outputHashMode","recursive"),("preferLocalBuild","1"),("system","builtin"),("unpack",""),("url","http://tarballs.nixos.org/stdenv/x86_64-unknown-linux-gnu/82b583ba2ba2e5706b35dbe23f31362e62be2a9d/busybox"),("urls","http://tarballs.nixos.org/stdenv/x86_64-unknown-linux-gnu/82b583ba2ba2e5706b35dbe23f31362e62be2a9d/busybox")])
```

For readability, you can convert the derivation to JSON by installing and
running [the `aterm2json` command](https://github.com/fzakaria/aterm2json).

</details>

### JSON derivation format

NOTE: The
[JSON derivation format](https://nix.dev/manual/nix/2.33/protocols/json/derivation)
is currently experimental and has not been stabilized at the time of writing.
There is a required
[`version` field](https://nix.dev/manual/nix/2.33/protocols/json/derivation/#version)
that defines how Nix understands the rest of the JSON.

Newer JSON derivation versions require newer versions of the `nix` binary. You
can check which JSON derivation versions are available to you by running
`nix --version` and visiting:
`https://nix.dev/manual/nix/<nix-version>/protocols/json/derivation`.

At the time of writing, `nix-ninja` generates its own unversioned derivations
(which seem to resemble version 3) using
[the format defined in `crates/nix-libstore/src/derivation.rs`](https://github.com/pdtpartners/nix-ninja/blob/8da02bd560f8bb406b82ae17ca99375f2b841b12/crates/nix-libstore/src/derivation.rs#L7-L46).
There is
[ongoing work to migrate to a community-maintained format](https://github.com/pdtpartners/nix-ninja/pull/40)
that defines a
[default version](https://github.com/nix-community/harmonia/blob/77fd6d9fc645f243434dbef3b919c693cc13e07e/harmonia-store-core/src/derivation/basic_derivation.rs#L49-L51)
used in generating JSON derivations.

#### Version 3 Notes

Since `inputSrcs` are just store paths, you can just refer to them by absolute
paths in your build process, or use environment variables like nixpkgs'
`stdenv.mkDerivation` (which sets `$src`).

On the other hand, `inputDrvs` must be referenced using "placeholders", which
are encoded values that point to outputs of `inputsDrvs`. More on placeholders
in the next section.

## Placeholders

Inputs that are outputs of other derivations can be referenced in process
creation fields via "placeholders". These are opaque values in the form of
`/<hash>`.

Note that placeholders existed before dynamic derivations:

```sh
nix-repl> builtins.placeholder "foo"
"/1x0ymrsy7yr7i9wdsqy9khmzc1yy7nvxw6rdp72yzn50285s67j5"
```

Under the hood, it's computed with this pseudo-code:

```python
# For regular input-addressed derivations
def placeholder(output_name: str) -> str:
    clear_text = f"nix-output:{output_name}"
    digest = sha256sum(clear_text)
    return nixbase32.encode(digest)
```

This is useful if you want to set an environment variable to what the output
path eventually resolves to. If you search for `builtins.placeholder` in
nixpkgs, you'll find many occurrences, e.g.:

```nix
KMODDIR = "${builtins.placeholder "out"}/kernel";
```

There are new placeholders, used to generate hashes for the new types of
derivations:

<!-- kylechui: Are CA and dynamic orthogonal? Is it possible to have a CA dynamic derivation? -->

- `nix-upstream-output:` for content-addressed derivations
- `nix-computed-output:` for dynamic derivations

They are computed differently (again pseudo-code):

```python
# For content-addressed derivations
def unknown_ca_output(drv_path: str, output_name: str) -> str:
    drv_name = drv_path.removesuffix('.drv')
    clear_text = f"nix-upstream-output:{drv_path.hash_part}:{drv_name}-{output_name}"
    digest = sha256sum(clear_text)
    return nixbase32.encode(digest)

# For dynamic derivations
def unknown_derivation(placeholder: str, output_name: str) -> str:
    # Take first 20 bytes of the input placeholder hash
    compressed = placeholder[:20]
    clear_text = f"nix-computed-output:{compressed}:{output_name}"
    digest = sha256sum(clear_text)
    return nixbase32.encode(digest)
```

## Dynamic outputs

When you depend on derivation-producing derivations, you need to use
[the `dynamicOutputs` field](https://nix.dev/manual/nix/2.33/protocols/json/derivation/#inputs_drvs_pattern1_pattern3_i1_dynamicOutputs)
to trigger the code path that handles dynamic derivations.

```json
// Mapping from derivation paths to objects defining the derivation outputs
"dynamicOutputs": {
  // NOTE: The path may or may not include the `/nix/store` prefix depending on
  // the JSON version
  "/path/to/derivation": {
    // Recursive object that specifies outputs from nested dynamic derivations
    "dynamicOutputs": {
      "/other/path/to/derivation": { ... }
    },
    // List of symbolic names for the outputs of the derivation
    "outputs": ["out"]
  }
},
```

This nested structure allows you to describe the output of derivation which was
generated by another derivation. Here's how you would create a placeholder that
references this dynamic output:

```python
# First get placeholder for "drv-out" output of the original derivation
drv_out_placeholder = unknown_ca_output("/nix/store/<hash>.drv", "drv-out")

# Then get placeholder for "out" output of the produced derivation
out_placeholder = unknown_derivation(drv_out_placeholder, "out")
```

## New Nix builtins

There are new builtin Nix functions to generate CA derivations and dynamic
derivations, but I'd recommend generating JSON derivations directly to avoid the
overhead of Nix evaluation. Nevertheless, I'll go over the new builtins for
completeness:

- `builtins.outputOf` returns a placeholder that references a output path of a
  derivation.
- `builtins.unsafeDiscardOutputDependency` is a leaky implementation detail that
  strips internal string metadata that refers to its output dependencies.

Let's walk through an example:

```nix
{ pkgs ? import <nixpkgs> {} }:

let
  caDrv = pkgs.stdenv.mkDerivation {
    name = "ca-example";
    # These fields indicate that this derivation is a CA derivation
    __contentAddressed = true;
    outputHashMode = "nar";
    outputHashAlgo = "sha256";

    outputs = [ "out" ];
    buildCommand = "...";
  };

  # Then a derivation that depends on a dynamic output.
  dynDrv = pkgs.stdenv.mkDerivation {
    name = "dynamic-example";
    # This creates placeholders using nix-computed-output:...
    # referencing the CA derivation's placeholders
    buildCommand = ''
      ${builtins.outputOf (builtins.unsafeDiscardOutputDependency caDrv) "out"}
    '';
  };

in { inherit caDrv dynDrv; }
```

Ideally the UX is `builtins.outputOf caDrv "out"` but I'll get into why the
other builtin is necessary later.

Let's first look at their JSON representations:

```json
{
  "/nix/store/w283xjf1174klb924fg0b6y5iwlhw1v0-ca-example.drv": {
    "name": "ca-example",
    /* ... */
    "env": {
      /* ... */
      "out": "/1rz4g4znpzjwh1xymhjpm42vipw92pr73vdgl6xs1hycac8kf2n9"
    },
    "outputs": {
      "out": {
        "hashAlgo": "sha256",
        "method": "nar"
      }
    }
  }
}
```

In the `ca-example.drv`, `$out` is a placeholder value that Nix will fill at
build-time, but you can use it regularly like `mkdir -p $out/bin`, etc.

```json
{
  "/nix/store/b7pcfk2d7knx76jjkb48hipywrkj0aak-dynamic-example.drv": {
    "name": "dynamic-example",
    /* ... */
    "env": {
      /* ... */
      "buildCommand": "/0g9wr256l3563hj4ivphq5wkyz7kby9h9sx17360q7hjaxjnvqj2\n"
    },
    "inputDrvs": {
      /* ... */
      "/nix/store/w283xjf1174klb924fg0b6y5iwlhw1v0-ca-example.drv": {
        "dynamicOutputs": {
          "out": {
            "dynamicOutputs": {},
            "outputs": ["out"]
          }
        },
        "outputs": []
      }
    }
  }
}
```

In the `dynamic-example.drv`, the `buildCommand` gets a `nix-computed-output`
placeholder based on the `dynamicOutputs` of the `ca-example.drv`.

Going back to `builtins.unsafeDiscardOutputDependency`, we can explore how it
works in the Nix repl:

```sh
nix-repl> caDrv
«derivation /nix/store/w283xjf1174klb924fg0b6y5iwlhw1v0-ca-example.drv»

nix-repl> builtins.outputOf caDrv "out"
error:
       … while calling the 'outputOf' builtin
         at «string»:1:1:
            1| builtins.outputOf caDrv "out"
             | ^

       … while evaluating the first argument to builtins.outputOf

       error: expected a string but found a set
```

What's going on? Turns out you must provide a string, here's the excerpt from
Nix's functional tests:

```bash
# We currently require a string to be passed, rather than a derivation
# object that could be coerced to a string. We might liberalise this in
# the future so it does work, but there are some design questions to
```

Okay let's try `caDrv.drvPath`:

```sh
nix-repl> builtins.outputOf caDrv.drvPath "out"
error:
       … while calling the 'outputOf' builtin
         at «string»:1:1:
            1| builtins.outputOf caDrv.drvPath "out"
             | ^

       … while evaluating the first argument to builtins.outputOf

       error: string '/nix/store/w283xjf1174klb924fg0b6y5iwlhw1v0-ca-example.drv'
              has a context which refers to a complete source and binary closure.
              This is not supported at this time.
```

I didn't understand what this meant, but using
`builtins.unsafeDiscardOutputDependency` fixes the issue, so let's a take a look
at that:

```sh
nix-repl> caDrv.drvPath
"/nix/store/w283xjf1174klb924fg0b6y5iwlhw1v0-ca-example.drv"

nix-repl> builtins.unsafeDiscardOutputDependency caDrv.drvPath
"/nix/store/w283xjf1174klb924fg0b6y5iwlhw1v0-ca-example.drv"
```

Huh? This is getting deep into the weeds, but strings in Nix have a "string
context" which holds metadata. `caDrv.drvPath` has a `DrvDeep` string context
that includes its entire build closure, which `builtins.outputOf` isn't happy
with.

```cpp
/**
 * Path to a derivation and its entire build closure.
 *
 * The path doesn't just refer to derivation itself and its closure, but
 * also all outputs of all derivations in that closure (including the
 * root derivation).
 *
 * Encoded in the form `=<drvPath>`.
 */
struct DrvDeep {
  /* ... */
}
```

`DrvDeep` string contexts are not supported by `builtins.outputOf` at the time
of writing this, but the source code does indicate that it may relax this
requirement in the future.

Anyway, you can explore the inner details by using `builtins.getContext`:

```sh
nix-repl> builtins.toJSON (builtins.getContext caDrv.drvPath)
"{\"/nix/store/w283xjf1174klb924fg0b6y5iwlhw1v0-ca-example.drv\":{\"allOutputs\":true}}"

nix-repl> builtins.toJSON (builtins.getContext (builtins.unsafeDiscardOutputDependency caDrv))
"{\"/nix/store/w283xjf1174klb924fg0b6y5iwlhw1v0-ca-example.drv\":{\"outputs\":[\"out\"]}}"
```

Internally, `"allOutputs": true` indicates a complete closure. After using
`builtins.unsafeDiscardOutputDependency`, it simplifies the context to just the
output. This is just a leaky implementation constraint where `builtins.outputOf`
needs clean derivation path references without full closure information.

## Command-line dynamic outputs

Finally, dynamic derivations brings a syntax to express dynamic outputs on the
command-line.

```md
/nix/store/<hash>-<name>.drv^foo.drv^bar.drv^out
|------------------------------------------| |-| inner deriving path output name
|----------------------------------| |-----| even more inner deriving path
output name |--------------------------| |-----| innermost store path output
name
```

This is represented by the equivalent `dynamicOutputs`:

```json
{
  "inputDrvs": {
    "/nix/store/<hash>-<name>.drv": {
      "dynamicOutputs": {
        "foo.drv": {
          "dynamicOutputs": {
            "bar.drv": {
              "dynamicOutputs": {},
              "outputs": ["out"]
            },
            "outputs": []
          },
          "outputs": []
        }
      },
      "outputs": []
    }
  }
}
```

And it is supported by `nix build` like so:

```sh
nix build "/nix/store/<hash>-<name>.drv^foo.drv^bar.drv^out"
```

## Conclusion

That's it! As far as I understand these are all the practical elements to
building using dynamic derivations. Let's summarize the main takeaways:

- Incremental compilation requires `ca-derivations` and `dynamic-derivations`
  experimental features
- The `text` hash mode allows a derivation to output a derivation.
- Derivations are traditionally serialized in ATerm format but I recommend
  utilizing the new JSON derivation format that can be written as an output.
- Placeholders are encoded values that reference `dynamicOutputs`
- `dynamicOutputs` is a structured object in `inputDrvs` of a derivation that
  can describe outputs of a derivation produced by another derivation.
- `builtins.outputOf` has quirks like `builtins.unsafeDiscardOutputDependency`
  to be aware of, but is used at eval-time to produce placeholders.

## Sources

- [Cpp Nix source](https://github.com/NixOS/nix/tree/master/src)
- [Nix manual](https://github.com/NixOS/nix/tree/master/doc/manual/source)
- [Sandstone - incremental haskell builds with dynamic derivations](https://github.com/obsidiansystems/sandstone)
- [Haskell Nix source in Obsidian System's fork](https://github.com/obsidiansystems/hnix-store/tree/derivation-work)
- [Tvix nix-compat crate source](https://code.tvl.fyi/tree/tvix/nix-compat)
- [Build Systems a la Carte](https://www.microsoft.com/en-us/research/wp-content/uploads/2018/03/build-systems.pdf)
