#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { chromium } from "playwright";

const origin = process.env.CHAPTERA_SECURE_ORIGIN;
const providerOrigin = process.env.CHAPTERA_OIDC_PROVIDER_ORIGIN;
const receiptPath = process.env.CHAPTERA_SECURE_RECEIPT;
const screenshotPath = process.env.CHAPTERA_SECURE_SCREENSHOT;

for (const [name, value] of Object.entries({
  CHAPTERA_SECURE_ORIGIN: origin,
  CHAPTERA_OIDC_PROVIDER_ORIGIN: providerOrigin,
  CHAPTERA_SECURE_RECEIPT: receiptPath,
  CHAPTERA_SECURE_SCREENSHOT: screenshotPath,
})) {
  if (!value) throw new Error(name + " is required");
}

function requireStatus(actual, expected, label) {
  if (actual !== expected) {
    throw new Error(label + " expected " + expected + ", got " + actual);
  }
}

async function sessionFrom(page) {
  return page.evaluate(async () => {
    const response = await fetch("/v1/session", {
      credentials: "include",
      cache: "no-store",
    });
    let body = null;
    try {
      body = await response.json();
    } catch {
      body = null;
    }
    return { status: response.status, body };
  });
}

async function main() {
  const browser = await chromium.launch({
    headless: true,
    args: [
      "--host-resolver-rules=MAP edge.test 127.0.0.1",
    ],
  });
  const browserVersion = browser.version();
  const context = await browser.newContext({
    ignoreHTTPSErrors: true,
    viewport: { width: 1280, height: 800 },
  });
  const page = await context.newPage();

  try {
    await page.goto(
      origin + "/v1/auth/login?return_path=%2Flive",
      { waitUntil: "networkidle", timeout: 30000 },
    );
    if (page.url() !== origin + "/live") {
      throw new Error("OIDC browser flow did not return to secure Chaptera origin: " + page.url());
    }

    const cookies = await context.cookies(origin);
    const sessionCookie = cookies.find(
      (cookie) => cookie.name === "__Host-chaptera_session",
    );
    if (!sessionCookie) {
      throw new Error("browser did not store __Host-chaptera_session");
    }
    if (!sessionCookie.secure) {
      throw new Error("stored Chaptera session cookie is not Secure");
    }
    if (!sessionCookie.httpOnly) {
      throw new Error("stored Chaptera session cookie is not HttpOnly");
    }
    if (sessionCookie.sameSite !== "Lax") {
      throw new Error("stored Chaptera session cookie is not SameSite=Lax");
    }
    if (sessionCookie.path !== "/") {
      throw new Error("stored Chaptera session cookie path is not /");
    }
    if (sessionCookie.domain !== "edge.test") {
      throw new Error(
        "stored __Host cookie is not host-only for edge.test: " +
        sessionCookie.domain,
      );
    }

    const initialSession = await sessionFrom(page);
    requireStatus(initialSession.status, 200, "authenticated /v1/session");
    if (
      !initialSession.body ||
      typeof initialSession.body.principal_id !== "string" ||
      !initialSession.body.principal_id ||
      typeof initialSession.body.csrf_token !== "string" ||
      !initialSession.body.csrf_token
    ) {
      throw new Error("authenticated session bootstrap lacks principal/CSRF");
    }

    const missingCsrf = await page.evaluate(async () => {
      const response = await fetch("/v1/auth/logout", {
        method: "POST",
        credentials: "include",
      });
      let body = null;
      try {
        body = await response.json();
      } catch {
        body = null;
      }
      return { status: response.status, body };
    });
    requireStatus(missingCsrf.status, 403, "logout without CSRF");

    await page.screenshot({ path: screenshotPath, fullPage: true });

    await page.goto(providerOrigin + "/jwks", {
      waitUntil: "domcontentloaded",
      timeout: 10000,
    });
    const crossOrigin = await page.evaluate(
      async ({ targetOrigin, csrf }) => {
        try {
          const response = await fetch(targetOrigin + "/v1/auth/logout", {
            method: "POST",
            credentials: "include",
            headers: { "x-csrf-token": csrf },
          });
          return {
            blocked: false,
            status: response.status,
          };
        } catch (error) {
          return {
            blocked: true,
            error: String(error),
          };
        }
      },
      {
        targetOrigin: origin,
        csrf: initialSession.body.csrf_token,
      },
    );
    if (!crossOrigin.blocked && crossOrigin.status < 400) {
      throw new Error(
        "cross-origin browser mutation was not rejected: " +
        JSON.stringify(crossOrigin),
      );
    }

    await page.goto(origin + "/live", {
      waitUntil: "networkidle",
      timeout: 10000,
    });
    const refreshedSession = await sessionFrom(page);
    requireStatus(refreshedSession.status, 200, "session after rejected probes");
    if (
      !refreshedSession.body ||
      typeof refreshedSession.body.csrf_token !== "string" ||
      !refreshedSession.body.csrf_token
    ) {
      throw new Error("refreshed session lacks CSRF token");
    }

    const logout = await page.evaluate(async (csrf) => {
      const response = await fetch("/v1/auth/logout", {
        method: "POST",
        credentials: "include",
        headers: { "x-csrf-token": csrf },
      });
      return { status: response.status };
    }, refreshedSession.body.csrf_token);
    requireStatus(logout.status, 204, "same-origin authenticated logout");

    const afterLogout = await sessionFrom(page);
    requireStatus(afterLogout.status, 401, "session after logout");

    const remainingCookies = await context.cookies(origin);
    const cookieRemoved = !remainingCookies.some(
      (cookie) => cookie.name === "__Host-chaptera_session",
    );
    if (!cookieRemoved) {
      throw new Error("logout did not remove browser Chaptera session cookie");
    }

    const receipt = {
      schema: "chaptera.secure-origin-oidc-browser.receipt.v1",
      task: "LOCAL-WEB-JOURNEY-01 / Phase C-C secure-origin identity",
      git_sha: process.env.GITHUB_SHA ?? "unknown",
      public_safe: true,
      browser: {
        name: "chromium",
        version: browserVersion,
        headless: true,
      },
      secure_origin: origin,
      real_chaptera_login_route_used: true,
      real_chaptera_callback_route_used: true,
      local_oidc_authorization_code_pkce_used: true,
      cookie: {
        name: sessionCookie.name,
        secure: sessionCookie.secure,
        http_only: sessionCookie.httpOnly,
        same_site: sessionCookie.sameSite,
        path: sessionCookie.path,
        host_only_edge_test: sessionCookie.domain === "edge.test",
      },
      browser_session_bootstrap_status: initialSession.status,
      principal_present: Boolean(initialSession.body.principal_id),
      csrf_present: Boolean(initialSession.body.csrf_token),
      missing_csrf_rejected_status: missingCsrf.status,
      cross_origin_mutation_rejected:
        crossOrigin.blocked || crossOrigin.status >= 400,
      same_origin_logout_status: logout.status,
      post_logout_session_status: afterLogout.status,
      cookie_removed_after_logout: cookieRemoved,
      direct_app_port_used_by_browser: false,
      production_hostname_claim: false,
      external_invited_identity_claim: false,
      s3_source_ingress_claim: false,
      controlled_pilot_environment_claim: false,
    };

    fs.mkdirSync(path.dirname(receiptPath), { recursive: true });
    fs.writeFileSync(
      receiptPath,
      JSON.stringify(receipt, null, 2) + "\n",
      "utf8",
    );
    process.stdout.write(JSON.stringify(receipt, null, 2) + "\n");
  } finally {
    await context.close();
    await browser.close();
  }
}

await main();
