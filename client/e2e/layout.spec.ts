import { expect, test } from "@playwright/test";

const items = Array.from({ length: 100 }, (_, i) => ({
  id: String(i),
  name: `Item ${i}`,
  updated_at: "2026-09-11",
}));

for (const viewport of [
  { width: 1440, height: 900 },
  { width: 1280, height: 720 },
  { width: 375, height: 812 },
  { width: 320, height: 640 },
]) {
  for (const colorScheme of ["dark", "light"] as const) {
    test(`${viewport.width} ${colorScheme}: cards and table flow with the page`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize(viewport);
      await page.emulateMedia({ colorScheme });
      await page.route("**/api/items", (route) =>
        route.fulfill({ json: items }),
      );
      await page.goto("/");
      await expect(page.locator(".card")).toHaveCount(100);
      for (const [url, rowSelector, containerSelector] of [
        ["/", ".card", ".cards"],
        ["/e2e/table.html", "tbody tr", ".table-scroll"],
      ]) {
        if (url !== "/") await page.goto(url);
        await expect(page.locator(rowSelector)).toHaveCount(100);
        const geometry = await page.evaluate(
          ({ rowSelector, containerSelector }) => {
            const container = document.querySelector(containerSelector)!;
            const rows = [...document.querySelectorAll(rowSelector)].map(
              (row) => row.getBoundingClientRect(),
            );
            return {
              firstTop: rows[0].top,
              visible: rows.filter(
                (row) =>
                  row.top >= (rowSelector === ".card" ? 48 : 0) &&
                  row.bottom <= innerHeight,
              ).length,
              innerScroll: container.scrollHeight - container.clientHeight,
              documentWidth: document.documentElement.scrollWidth,
              maxHeight: getComputedStyle(container).maxHeight,
            };
          },
          { rowSelector, containerSelector },
        );
        expect(geometry.documentWidth).toBe(viewport.width);
        expect(geometry.innerScroll).toBeLessThanOrEqual(1);
        expect(geometry.maxHeight).toBe("none");
        expect(geometry.firstTop).toBeLessThanOrEqual(url === "/" ? 160 : 65);
        expect(geometry.visible).toBeGreaterThanOrEqual(url === "/" ? 7 : 15);
        await testInfo.attach(`${rowSelector}-geometry`, {
          body: JSON.stringify(geometry),
          contentType: "application/json",
        });
        await testInfo.attach(`${rowSelector}-screen`, {
          body: await page.screenshot(),
          contentType: "image/png",
        });
        const first = page.locator(rowSelector).first();
        const before = (await first.boundingBox())!.y;
        await page.mouse.move(viewport.width / 2, viewport.height / 2);
        await page.mouse.wheel(0, 600);
        await expect
          .poll(() => page.evaluate(() => scrollY))
          .toBeGreaterThan(500);
        expect((await first.boundingBox())!.y).toBeLessThan(before - 500);
        if (url !== "/") {
          expect(
            (await page.locator("th").first().boundingBox())!.y,
          ).toBeLessThan(0);
          await expect(page.locator("th").first()).toHaveCSS(
            "position",
            "static",
          );
          if (viewport.width < 768) {
            const region = page.getByRole("region");
            await region.focus();
            await page.keyboard.press("ArrowRight");
            await expect
              .poll(() => region.evaluate((el) => el.scrollLeft))
              .toBeGreaterThan(0);
          }
        }
      }
    });
  }
}

test("long names, focus, search and loading/empty/error keep the list usable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 320, height: 640 });
  let response: "loading" | "empty" | "error" | "success" = "loading";
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route("**/api/items", async (route) => {
    if (response === "loading") await pending;
    await route.fulfill({
      status: response === "error" ? 500 : 200,
      json:
        response === "empty"
          ? []
          : [{ ...items[0], name: "LongName".repeat(30) }],
    });
  });
  await page.goto("/");
  await expect(page.getByText("読み込み中…")).toBeVisible();
  response = "empty";
  release();
  await expect(
    page.getByText("項目がありません", { exact: true }),
  ).toBeVisible();
  response = "error";
  await page.reload();
  await expect(page.getByText("読み込みに失敗しました")).toBeVisible();
  response = "success";
  await page.getByRole("button", { name: "再試行" }).click();
  await expect(page.locator(".card")).toHaveCount(1);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(
    320,
  );
  const search = page.getByRole("searchbox");
  await search.focus();
  await page.keyboard.press("Tab");
  await expect(page.locator(".card")).toBeFocused();
  await expect(page.locator(".card")).toHaveCSS("outline-style", "solid");
  await search.fill("missing");
  await expect(page.getByRole("status")).toHaveText("一致する項目がありません");
  await search.fill("");
  await expect(page.locator(".card")).toHaveCount(1);
});
