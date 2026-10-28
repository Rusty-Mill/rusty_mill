// Browser check of signing in: with no token, or a refused one, the board asks for it and carries on.
//
//   D=$(mktemp -d); rusty_mill_board --settings-dir $D --http 127.0.0.1:8799 2>$D/log &
//   TOKEN=$(cat $D/first-admin.token)
//   node tests/ui/signin.mjs "$(npm root -g)/playwright" http://127.0.0.1:8799 "$TOKEN"
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)(process.argv[2]);
const [BASE, TOKEN] = [process.argv[3], process.argv[4]];

const failures = [];
const check = (ok, what) => { console.log(`${ok ? "ok  " : "FAIL"} ${what}`); if (!ok) failures.push(what); };
const signedInAs = (page) => page.waitForFunction(() => document.querySelector("#actor").value === "admin");

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1300, height: 900 } });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

// No token: the sign-in box, not a silently empty board.
await page.goto(BASE);
await page.waitForSelector("#signin[open]");
check((await page.locator("#signin-why").innerText()).includes("needs a token"), "with no token the board asks for one");
await page.keyboard.press("Escape");
check(await page.locator("#signin").evaluate((d) => d.open), "Escape does not dismiss it");

// A wrong token is refused visibly; the right one lets every waiting call through.
await page.fill("#signin-token", "wrong");
await page.click("#signin button");
await page.waitForFunction(() => document.querySelector("#signin-why").textContent.includes("refused"));
check(await page.locator("#signin").evaluate((d) => d.open), "a refused token is said so and asked again");
await page.fill("#signin-token", TOKEN);
await page.click("#signin button");
await signedInAs(page);
check(!(await page.locator("#signin").evaluate((d) => d.open)), "the right token closes the box");
check(await page.locator("#signout").isVisible(), "and offers Sign out");
await page.fill("#title", "signed in");
await page.click("#add");
await page.locator(".card", { hasText: "signed in" }).waitFor();
check(true, "board calls work after signing in");

// The token is kept in this browser, and never in the address bar.
await page.reload();
await signedInAs(page);
check(!(await page.locator("#signin").evaluate((d) => d.open)), "a reload stays signed in");
const linked = await browser.newPage();   // its own storage: only the link can sign it in
await linked.goto(`${BASE}/#token=${TOKEN}`);
await signedInAs(linked);
check((await linked.evaluate(() => location.hash)) === "", "a #token= link is taken and dropped from the address");
await linked.close();

// Sign out forgets it.
await page.click("#signout");
await page.waitForSelector("#signin[open]");
check(await page.evaluate(() => localStorage.getItem("token")) === null, "Sign out forgets the token and asks again");

check(errors.length === 0, `no page errors ${errors.join("; ")}`);
await browser.close();
if (failures.length) { console.error(`\n${failures.length} check(s) failed`); process.exit(1); }
console.log("\nall browser checks passed");
