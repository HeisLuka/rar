# VISREG-01 structured visual regression oracle

This tool compares public-safe, source-neutral golden/candidate projection fixtures.

It deliberately separates:

- **structural** changes: added/removed pages or objects, object-kind changes;
- **geometry** changes: page size or object bounds;
- **text_layout** changes: Story origin, text, or line placement;
- **render_only** changes: normalized render artifact changes when structure/geometry/text are otherwise identical.

Pixel equality is not treated as semantic truth. A future pixel-diff adapter may add a render signal, but it must remain secondary to the structured projections.

## Fixture contract

Each fixture uses `chaptera.visreg.fixture.v1` and contains stable page/object identities. Text objects may carry `story_id`, scalar text and line boxes. Each object may carry a normalized `render` record (for example a backend-independent digest).

The report uses `chaptera.visreg.report.v1`, is deterministically sorted, contains no timestamps or machine-local paths, and backreferences differences to `page_id`, `node_id`, and `story_id` when available.

## CLI

```bash
python tools/visreg/visreg.py \
  --golden tools/visreg/fixtures/golden.json \
  --candidate tools/visreg/fixtures/geometry-regression.json \
  --out target/visreg/report.json \
  --expect-stage geometry
```

Without `--expect-stage`, a difference returns exit code 1. Invalid input returns 2.

The GitHub Actions workflow proves:

1. identical inputs classify as `clean`;
2. an intentional bounds change classifies as `geometry`;
3. an intentional Story/line change classifies as `text_layout`;
4. a render digest-only change classifies as `render_only`;
5. two runs over identical inputs produce byte-identical normalized reports.
