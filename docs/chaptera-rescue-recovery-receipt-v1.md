# Chaptera Rescue source-free recovery receipt v1

This is the public product boundary between the authorized local/native recovery producer and Chaptera Rescue.

The public repository does **not** receive damaged PUB bytes, recovered text, local paths, customer identity, private parser state, or licensed/native runtime state. It receives only a bounded receipt describing the already-proven recovery result.

## Current P0 witnesses

- natural partial-CFB witness: `koronavirus-ispravlennaja!.pub` — SHA-256 `6eb0a85afb42d329e2241ce060ec3b6957ecd429a5122cfc908b4621ab4884b8`;
- healthy fail-closed control: `51318.pub` — SHA-256 `3ab75a6a9196e0a51fc9b0aa759459501c71d313030c06652aabffbae0a2ab09`;
- optional partial fail-closed control: `zbirka_ato.pub` — SHA-256 `2173c7dd2349c6ced2f838df458cc2503e15606d073cc8d6146706d372dd64d8`.

## Product outcomes

The consumer maps producer routes to exactly four Chaptera Rescue outcomes:

- `bounded_recovered`;
- `partially_recovered`;
- `manual_review`;
- `unsupported/no_safe_recovery`.

A healthy control may use producer route `diagnostic_only`, but the product outcome remains `unsupported/no_safe_recovery`; it is never promoted to a recovery success.

## Fail-closed invariants

- source hash before and after must equal the pinned witness hash;
- `fabricated_bytes == 0`;
- `silent_drops == 0`;
- bounded/partial recovered artifacts are exact and carry verified source ranges;
- partial recovery has explicit known loss;
- a partial control cannot become `bounded_recovered`;
- a native PUB artifact is impossible unless a separate native-validation receipt is `valid`;
- private/raw content fields are rejected by the schema.

Validate and emit the public product receipt:

```bash
python tools/validate_rescue_recovery_receipt.py \
  local-source-free-producer-receipt.json \
  --output chaptera-rescue-product-validation.json
```

The resulting product receipt is safe to retain in Rar and is intended to become the input boundary for the standalone Chaptera Rescue application. It does not execute recovery and does not define new repair semantics.
