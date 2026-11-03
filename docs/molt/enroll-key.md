# crust molt enroll-key

Molt ships ZFS, and later other drivers, as kernel modules built outside the
kernel. They are signed with the **Crustates module-signing key**. With Secure
Boot on, the kernel only loads modules signed by a key the machine trusts, so
you enrol the Crustates certificate once as a Machine Owner Key (MOK).

Without Secure Boot you don't need this: the modules load anyway.

## Before you start

- You need the machine's own screen and keyboard: enrolment finishes in a
  firmware screen during boot.
- You need `crust` logged in to the machine (see [Install](../install.md)).
- The certificate is in the image at
  `/usr/share/crustates/keys/crustates-module-signing.der`.

## Steps

1. Queue the key from your computer:

    ```sh
    crust molt enroll-key
    ```

    The machine checks that Secure Boot is on and that the key is not enrolled
    yet, then prints the certificate's SHA-256 fingerprint and a one-time
    password. Compare the fingerprint with the one in the release notes.

    Images from local and pull request builds are signed with a throwaway key
    whose subject says `throwaway, not for release`; the command refuses to
    enrol it.

2. Reboot the machine. Before Molt starts, the blue **MOK management** screen
   appears. Choose **Enroll MOK**, then **Continue**, then **Yes**, type the
   one-time password, and choose **Reboot**.

3. After the reboot, `crust molt enroll-key` says the key is already
   enrolled, and ZFS pools work.

Only someone at the machine can finish step 2: a new boot key always needs
physical presence.
