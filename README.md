# Pakstak

Pakstak is a simple unprivileged local container manager and an experiment in "containers for applications" and "universal packaging".

Instead of defining new repository and package formats, it is built around the OCI ecosystem and tools.

Bubblewrap is used to stack layers/images and provide an isolated environment for the app.

## Goals

- Work on any Linux distribution that has non-setuid Bubblewrap
- Reasonable and customizable isolation
- Require no special privileges (be rootless)
- Well-supported backing format (OCI images)
- Few runtime and build-time dependencies
- Simplicity, as in "easy to understand how it works", not as in "easy to use"

## Non-Goals

- Integrating with non-universally available software
- Providing a custom image format and/or tools for building container images
- Feature completeness

## Status

This is experimental software. Expect breaking changes.

## Runtime Dependencies

If compiled statically:

- Non-setuid version of Bubblewrap
- CA certificates

If the build is not static, additionally:

- libc
- libgcc and/or other C toolchain dependencies

## Basic Usage

Install an image:

```sh
pakstak install my_alpine registry-1.docker.io/library/alpine:latest
```

Run a command from an installed container:

```sh
pakstak run my_alpine -- /bin/sh
```

Pakstak uses OCI images but does not apply image configuration such as
`ENTRYPOINT`, `CMD`, `ENV`, `WORKDIR`, or `USER`. Specify the command and any required
environment or working-directory options explicitly.

Provided by default:

- a read-only root filesystem view based on the images
- cleared environment variables
- isolated namespaces (including the network)
- fresh `/proc`, `/dev`, and `/tmp` mounts

Host files and networking must be enabled explicitly with Bubblewrap options.

Arguments after the first `--` are passed as-is to Bubblewrap, so you can define your own
bindings and other sandbox parameters, for example:

```sh
pakstak run my_alpine -- --share-net --bind "$HOME" /mnt -- /bin/sh
```

Update:

```sh
pakstak update
```

Pakstak uses per-user storage. The `PAKSTAK_STORAGE_PATH` environment variable
can override its location; otherwise it uses `$XDG_DATA_HOME/pakstak`, falling
back to `$HOME/.local/share/pakstak` when `XDG_DATA_HOME` is unset or empty.

## Image Sources

Works with most public OCI-compliant container registries and images built with
standard tools such as Docker, Podman, or Buildah.
There is also [Pakstash](https://github.com/pakstash/collection), a collection of images
for desktop apps compatible with Pakstak. Contributions are welcome.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
