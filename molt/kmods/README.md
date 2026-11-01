# Out-of-tree kernel modules

Each module has a folder here with a `build.sh` and its own stage in
`molt/Containerfile`, built on the `kmod-base` stage (the base image plus the
`kernel-devel` that matches `molt/kernel.lock`):

```dockerfile
FROM kmod-base AS kmod-<name>
COPY molt/kmods/<name>/ /kmods/<name>/
RUN /kmods/<name>/build.sh            # installs into /out, modules in /out/usr/lib/modules/<kernel>/extra
RUN --mount=type=secret,id=module-signing-key,required=true \
    --mount=type=secret,id=module-signing-cert,required=true \
    /kmods/sign.sh                    # signs every .ko under /out
```

and the `molt` stage copies `/out` in with `COPY --from=kmod-<name> /out/ /`.

`build.sh` must fail with a clear message when the module does not support the
kernel in `kernel.lock`. `sign.sh` signs with the Crustates module-signing key
(see `docs/molt/enroll-key.md`); `cargo xtask molt build` passes the key as a
build secret, so it never lands in an image layer.
