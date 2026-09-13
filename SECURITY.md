# Security policy

StateChronicle handles ownership, balances, marketplace state, and signed
history. Do not report a suspected vulnerability in a public issue when it
could expose an exploit or player data.

## Reporting

Use the repository's private security-advisory workflow (GitHub Security
Advisories) or contact the maintainers privately through the repository owner.
Include the affected version/commit, a minimal reproduction, impact, and any
known mitigations. Do not include private keys, production credentials, or
player-identifying data.

Maintainers will acknowledge a report within seven days, triage severity, and
coordinate a fix and disclosure timeline with the reporter. Emergency issues
affecting asset or currency integrity may require an immediate write freeze;
operators should follow the recovery and signer-compromise procedures in
`TODO.md` while a patch is prepared.

## Supported versions

Only the latest release line receives security fixes until a supported-version
policy is published. Consumers should pin a locked dependency graph and run
`cargo audit` before deploying. Maintainer ownership and emergency dependency
patch procedures are documented in `DEPENDENCY_MAINTENANCE.md`.
