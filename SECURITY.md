# Security policy

## Supported versions

Only the newest tagged Clannon local alpha receives security fixes. Versions are
`0.y.z`; compatibility may change between minor versions before 1.0. The `main`
branch and older alphas are development or historical code, not supported
security releases.

There is no supported public alpha until the repository has both a tagged
release and private vulnerability reporting enabled.

## Reporting a vulnerability

For a supported release, use **Security → Report a vulnerability** in the
Clannon GitHub repository. Include the affected version, host and Podman
versions, reproduction steps, impact, and whether the issue left a container or
host process behind. Remove private capability URLs, bearer values, terminal
contents, and other secrets that are not essential to the report.

Do not open a public issue for an unpatched access-control bypass, container
escape, artifact compromise, or other exploitable vulnerability. If the private
reporting control is unavailable, the repository has not met its own release
contract; do not publish exploit details while the maintainers establish a
private channel.

This local alpha is maintained on a best-effort basis without a response-time or
embargo SLA. Maintainers will validate the report, determine affected versions,
and coordinate a fix and disclosure when evidence permits.

## Security boundary

Clannon binds only to numeric loopback addresses and protects runtime APIs with
a fresh process capability. Treat the complete printed URL as a secret: anyone
who can read it can control this Clannon process. Do not paste it into public
reports, shell transcripts, screenshots, or shared browser sessions.

Guest commands run only in rootless Podman containers with outbound networking
disabled and explicit resource limits. This reduces risk; it is not a hardened
hostile-code or multi-tenant sandbox. Clannon has no accounts, remote access,
TLS, persistence, encrypted evidence store, signed updates, or recovery system.
It should run as an unprivileged local user, never through `sudo`, on a host where
that user's Podman boundary is acceptable.

The default image is pulled by Podman from its configured registry when absent.
The published checksum detects archive corruption but is not a digital signature
or an independent trust channel, and it does not authenticate or freeze the guest
image tag. Abrupt process or host termination can leave a Clannon-named container
that must be inspected and removed explicitly.

The candidate archive includes a locked dependency inventory, not a
standards-compliant SBOM, third-party license bundle, independent security audit,
or signed provenance. Release acceptance requires a current advisory review, but
the candidate workflow does not yet automate one.

Useful private reports include:

- access to an API or terminal without the current capability;
- acceptance of a non-loopback bind, foreign Host, or disallowed Origin;
- guest execution on the host or a rootless-container escape;
- unexpected guest outbound network access or bypassed resource limits;
- leaked containers, terminal proxy processes, secrets, or retained evidence;
- a release archive or checksum that differs from the tagged source.

Feature requests and limitations already documented in `PROJECT.md` are not
security vulnerabilities by themselves.
