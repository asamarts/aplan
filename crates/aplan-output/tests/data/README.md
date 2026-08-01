# Vendored test data

`sarif-2.1.0.schema.json` is the official SARIF 2.1.0 JSON Schema, from
[oasis-tcs/sarif-spec](https://github.com/oasis-tcs/sarif-spec)
(`sarif-2.1/schema/sarif-schema-2.1.0.json`).

It is vendored rather than fetched so the test that validates `--format sarif`
against it is hermetic: a gate that needs the network is a gate that fails when
the network does, and this one runs on every push.
