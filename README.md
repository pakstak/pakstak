# Pakstak

Pakstak is an experiment in "containers for applications" and "universal packaging".

It is inspired by Flatpak-style application isolation, but aims to be much simpler and to build on the shoulders of giants: OCI and Bubblewrap.

## Goals

- Work on any Linux distribution that has non-setuid Bubblewrap
- Reasonable and customizable isolation
- Require no special privileges (rootless)
- Well-supported backing format (OCI images)
- Few runtime and build-time dependencies
- Simplicity, as in "easy to understand how it works", not as in "easy to use"

## Non-Goals

- Replacing Flatpak
- Integrating with non-universally available software
- Providing a custom image format and/or tools for building container images
- Feature completeness

## Runtime Dependencies

If compiled statically:

- Non-setuid version of Bubblewrap
- CA certificates

If the build is not static, additionally:

- Libc

## Basic Usage

Install an image:

```sh
pakstak install my_alpine registry-1.docker.io/library/alpine:latest
```

Run a command from an installed container:

```sh
pakstak run my_alpine -- /bin/sh
```

Note that arguments after the first `--` are passed as-is to Bubblewrap, so
you can define your own bindings and other sandbox parameters, for example:

```sh
pakstak run my_alpine -- --share-net --bind "$HOME" /mnt -- /bin/sh
```

Update:

```sh
pakstak update
```

Currently, it only uses per-user storage: `$HOME/.var/pakstak`,
which is configurable through the `PAKSTAK_STORAGE_PATH` environment variable.

## Image Sources

You can use any public OCI-compliant container registry and build images with
standard tools such as Docker, Podman, or Buildah.
There is also [Pakstash](https://github.com/pakstash/collection), a collection of images
for desktop apps compatible with Pakstak. Contributions are welcome.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
