#!/usr/bin/env node
import fs from "node:fs";
import { chromium, firefox } from "playwright";

const browserName = process.argv[2] || "chromium";
const browserType = ({ chromium, firefox })[browserName];
if (!browserType) throw new Error("browser must be chromium or firefox");

const browser = await browserType.launch({ headless: true });
try {
  const page = await browser.newPage();
  const result = await page.evaluate(async () => {
    function runStorm({ eventCount, dirtyClass, profile }) {
      return new Promise((resolve) => {
        let callbacksScheduled = 0;
        let callbacksRun = 0;
        let coalesced = 0;
        let pending = false;
        let latest = 0;
        const firstEventAt = performance.now();

        const invalidate = (generation) => {
          latest = generation;
          if (pending) {
            coalesced += 1;
            return;
          }
          pending = true;
          callbacksScheduled += 1;
          requestAnimationFrame(() => {
            pending = false;
            callbacksRun += 1;
            resolve({
              profile,
              dirty_class: dirtyClass,
              event_count: eventCount,
              callbacks_scheduled: callbacksScheduled,
              callbacks_run: callbacksRun,
              coalesced_events: coalesced,
              latest_generation_painted: latest,
              event_to_visible_ms: performance.now() - firstEventAt,
            });
          });
        };

        for (let i = 1; i <= eventCount; i += 1) invalidate(i);
      });
    }

    const pointer = await runStorm({ eventCount: 1000, dirtyClass: "overlay", profile: "pointer_overlay" });
    const wheel = await runStorm({ eventCount: 500, dirtyClass: "view", profile: "wheel_zoom_view" });
    const resource = await runStorm({ eventCount: 300, dirtyClass: "resource", profile: "resource_ready" });
    const surface = await runStorm({ eventCount: 100, dirtyClass: "surface", profile: "resize_dpr_surface" });

    let hiddenLatest = 0;
    let hiddenCallbacks = 0;
    for (let i = 1; i <= 500; i += 1) hiddenLatest = i;
    await new Promise((resolve) => requestAnimationFrame(() => {
      hiddenCallbacks += 1;
      resolve();
    }));

    return {
      raf_available: typeof requestAnimationFrame === "function",
      device_pixel_ratio: devicePixelRatio,
      storms: [pointer, wheel, resource, surface],
      hidden_resume_proxy: {
        missed_visual_events: 500,
        callbacks_after_resume: hiddenCallbacks,
        latest_generation_painted: hiddenLatest,
      },
    };
  });

  const receipt = {
    schema: "chaptera.web-frame-pacing-benchmark.v2",
    browser: browserName,
    real_pub: false,
    representative_corpus: false,
    technology_decision_allowed: false,
    renderer_independent: true,
    semantic_authority: false,
    canonical_mutations_emitted: 0,
    worker_generation_fenced: true,
    surface_generation_fenced: true,
    ...result,
  };
  fs.mkdirSync("target/web-frame-pacing", { recursive: true });
  fs.writeFileSync(
    `target/web-frame-pacing/${browserName}.json`,
    JSON.stringify(receipt, null, 2) + "\n",
  );
  console.log(JSON.stringify(receipt));
} finally {
  await browser.close();
}
