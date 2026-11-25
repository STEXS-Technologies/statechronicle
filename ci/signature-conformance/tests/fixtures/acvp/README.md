# NIST ACVP signature vectors

These binary files come from the public [NIST ACVP Server vectors](https://github.com/usnistgov/ACVP-Server/tree/975de31eb83d87039ec88934fdc47d8c312b892d/gen-val/json-files) at revision `975de31eb83d87039ec88934fdc47d8c312b892d`. Each vector uses the pure external interface and an empty context. The tests exercise the provider adapters with the original ACVP messages. StateChronicle transcript interoperability is tested separately.

| Algorithm | Vector | ACVP directory | Test group | Test case |
| --- | --- | --- | --- | --- |
| ML-DSA-65 | Valid | `ML-DSA-sigGen-FIPS204` | 3 | 44 |
| ML-DSA-65 | Invalid | `ML-DSA-sigVer-FIPS204` | 3 | 39 |
| SLH-DSA-SHA2-128s | Valid | `SLH-DSA-sigGen-FIPS205` | 19 | 162 |
| SLH-DSA-SHA2-128s | Invalid | `SLH-DSA-sigVer-FIPS205` | 19 | 255 |

For valid cases, the messages come from `prompt.json` and the signatures from `expectedResults.json`. The ML-DSA verification key was derived from the published expanded test signing key with RustCrypto. The SLH-DSA verification key consists of the final 32 bytes of the published test signing key, encoding `PK.seed || PK.root`. For invalid cases, the message, public key, and signature come from `prompt.json`, and `expectedResults.json` gives `testPassed: false`.
