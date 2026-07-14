# Security Policy

rustyclinic handles protected health information. Security reports are taken
seriously and handled with priority.

## Reporting a vulnerability

**Do not open a public issue.** Instead, either:

- use GitHub's private vulnerability reporting ("Report a vulnerability" under
  the Security tab), or
- email **fomoroller@tutamail.com** with details and reproduction steps.

You can expect an acknowledgement within 72 hours. Please allow a reasonable
disclosure window for a fix before publishing details.

## Scope

Especially interested in: authentication/session flaws, tenant-isolation
bypasses (cross-facility data access), audit-chain tampering, package
signature bypasses, PHI exposure through sync or export paths, and SQL
injection in repository code.

## Supported versions

Pre-1.0, only the latest `main` is supported. There is no production
deployment yet — see the status note in the README.
