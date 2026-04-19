# Security Policy

## Supported Versions

Security fixes are applied to the **latest published version** on crates.io. Older versions do not receive backported patches.

| Version | Supported |
|---|:---:|
| 0.3.x (latest) | ✅ |
| 0.2.x and below | ❌ |

---

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.** Public disclosure before a fix is available puts all users at risk.

### How to report

Send an email to **[ankitchaubey.dev@gmail.com](mailto:ankitchaubey.dev@gmail.com)** with the subject line:

```
[ferogram SECURITY] <short description>
```

Include in your report:

- A description of the vulnerability
- Steps to reproduce or a proof-of-concept
- The affected version(s)
- Your assessment of the impact
- Your name or handle (optional, for credit)

### What to expect

- **Acknowledgement** within 48 hours
- **Assessment** and severity classification within 5 business days
- **Fix or mitigation** communicated privately before any public disclosure
- **Credit** in the changelog and advisory for reporters who wish it

---

## Scope

This policy covers the `ferogram`, `ferogram-mtproto`, and `ferogram-crypto` crates. Issues in transitive dependencies should be reported upstream to those projects.

### In scope

- Authentication bypass or session hijacking
- Cryptographic weaknesses in the MTProto implementation
- Memory safety issues (use-after-free, buffer overflow, etc.)
- Denial of service via crafted server responses
- Unintended data leakage from session storage

### Out of scope

- Telegram's own infrastructure or protocol design
- Vulnerabilities in user code that happens to use ferogram
- Social engineering attacks

---

## Disclosure Policy

Once a fix is published to crates.io, a public security advisory will be created on GitHub. The advisory will credit the reporter unless they request anonymity.

We follow a **coordinated disclosure** model. We ask reporters to allow at least **7 days** after a fix is released before publishing independent writeups, to give users time to upgrade.
