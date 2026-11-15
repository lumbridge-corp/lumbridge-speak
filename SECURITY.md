# Security

Report security bugs by email to security@lumbridgecorp.com, or use "Report a vulnerability" on this
repo's Security tab. Do not open a public issue with exploit details.

Include the commit or version you run, your OS, whether the problem needs local or network access,
steps to reproduce, and the impact you expect. Remove tokens from logs and examples.

The latest release and the latest `main` are supported. We answer in the same email thread. There is no
fixed response deadline.

## What to expect from this program

- The front has no authentication. It listens on loopback by default; anyone who can reach the port can
  make it speak and use the model server behind it. Do not expose it to a network you do not trust.
- It stores no audio and no text.
- `speak --on NAME` sends a bearer token, read from the file named in the settings, to the machine you
  configured. Keep that file private (mode 0600).
- The model server (Magpie on NeMo-Speech.cpp) is a separate program; report its problems to its owners.
