# OptChat native-tls patch

This is the published native-tls 0.2.18 source, with its original MIT/Apache-2.0
notices retained. The registry archive was checked against Cargo.lock before
copying it. The workspace patches this dependency locally because its public
builder does not expose OpenSSL system-policy configuration.

Upstream archive SHA256: `465500e14ea162429d264d44189adc38b199b62b1c21eea9f69e4b73cb03bbf2`.

On Linux with OpenSSL 1.1.1 or newer and the Fedora/RHEL OpenSSL policy file,
the connector reapplies OpenSSL's loaded `system_default` configuration after
the upstream builder has set its cipher list and protocol bounds. This restores
the administrator's ciphers, TLS versions, groups, signature algorithms and
security level. Explicit caller protocol restrictions are intersected with the
system bounds. Configuration failure returns an error; it does not fall back to
the upstream cipher list. The connector's certificate verification mode is
preserved even if the loaded configuration requests `VerifyMode=-Peer`;
hostname verification is still applied by native-tls at connection time.

Other operating systems and Linux systems without that policy file keep the
upstream behavior. OpenSSL stays dynamically linked to the distribution library.

The local changes are confined to build.rs and src/imp/openssl.rs. Review this
patch when upgrading native-tls, and remove the local copy when the upstream
API supports applying the system policy. OpenSSL's
[SSL_CTX_config](https://docs.openssl.org/3.5/man3/SSL_CTX_config/) API applies the
loaded configuration; [SSL_CONF_cmd](https://docs.openssl.org/3.5/man3/SSL_CONF_cmd/)
documents its crypto and verification options. TLS qualification uses the actual
client transports under Fedora DEFAULT and a stricter isolated configuration.
