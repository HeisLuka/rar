# OFFICEART-CONNECTOR-RULE-READ-01

This hosted-safe observer reuses the existing bounded `pub-escher`
`parse_officeart_stream` grammar.

It recognizes the public MS-ODRAW record identities:

- `OfficeArtSolverContainer` / `0xF005`;
- `OfficeArtFConnectorRule` / `0xF012`, exact 24-byte payload.

Each connector rule receipt preserves record/payload provenance and raw
`ruid`, `spidA`, `spidB`, `spidC`, `cptiA`, and `cptiB`.
Unknown solver siblings stay raw spans.

The same traversal exposes connector shapes only from the raw
`OfficeArtFSP.fConnector` flag and preserves the FSP `spid` and flags.

## Semantic firewall

The observer does not map OfficeArt SPIDs to Publisher Oids or Chaptera
NodeIds. It does not normalize connection-site indexes to Publisher COM
numbering, infer z-order, reroute connectors, or promote a connector graph.

Those joins remain with `PUB-T-848 / CONNECTOR-GRAPH-AUTH-01` and its native
Publisher causal experiment.

The CLI consumes a raw admitted OfficeArt stream and emits only logical stream
identity and source spans, never local file paths or document bytes.
