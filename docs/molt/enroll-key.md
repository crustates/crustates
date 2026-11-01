# crust molt enroll-key

> **Status:** the `crust molt enroll-key` command does not exist yet. Until it
> does, follow the manual steps below.

Molt ships ZFS, and later other drivers, as kernel modules built outside the
kernel. They are signed with the **Crustates module-signing key**. With Secure
Boot on, the kernel only loads modules signed by a key the machine trusts, so
you enrol the Crustates certificate once as a Machine Owner Key (MOK).

Without Secure Boot you don't need this: the modules load anyway.

## Before you start

- You need the machine's own screen and keyboard: enrolment finishes in a
  firmware screen during boot. Molt has no SSH.
- The certificate is in the image at
  `/usr/share/crustates/keys/crustates-module-signing.der`.
- Check that it is the release certificate. Its SHA-256 fingerprint must match
  the one in the release notes:

  ```sh
  openssl x509 -inform DER -noout -subject -fingerprint -sha256 \
    -in /usr/share/crustates/keys/crustates-module-signing.der
  ```

  Local and pull request builds are signed with a throwaway key whose subject
  says `throwaway, not for release`. Never enrol that one.

## Steps

1. Check that Secure Boot is on:

   ```sh
   mokutil --sb-state
   ```

   If it says `SecureBoot disabled`, stop here: nothing to enrol.

2. Queue the certificate for enrolment. `mokutil` asks for a one-time password
   that you type again in step 3:

   ```sh
   mokutil --import /usr/share/crustates/keys/crustates-module-signing.der
   ```

3. Reboot. Before Molt starts, the blue **MOK management** screen appears.
   Choose **Enroll MOK**, then **Continue**, then **Yes**, type the password
   from step 2, and choose **Reboot**.

4. After the reboot, check:

   ```sh
   mokutil --test-key /usr/share/crustates/keys/crustates-module-signing.der
   modprobe zfs
   ```

   `mokutil` reports the key as already enrolled, and `modprobe zfs` loads
   without an error.

## What the command will do

`crust molt enroll-key` will check the Secure Boot state and the certificate
fingerprint, queue the certificate (step 2) through the API, and tell you
what to do on the next boot. Steps 3 and 4 stay manual, because only someone
at the machine may approve a new boot key.
