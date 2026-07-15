"""
Download the GECK Wiki (geckwiki.com) for Fallout 3 / New Vegas modding reference.

Uses nodriver (undetected Chrome) to bypass Cloudflare Turnstile, then crawls
all wiki pages via Special:AllPages and saves each as HTML.
"""

import asyncio
import re
import sys
import time
from pathlib import Path
from urllib.parse import unquote, urljoin

import nodriver as uc
from bs4 import BeautifulSoup

WIKI_BASE = "https://geckwiki.com"
INDEX_URL = f"{WIKI_BASE}/index.php"

REQUEST_DELAY = 0.7


def log(msg):
    print(msg, flush=True)


def title_to_filename(title: str) -> str:
    safe = title.replace("/", "-").replace("\\", "-").replace(":", " -")
    safe = re.sub(r'[<>"|?*]', '_', safe)
    safe = re.sub(r'\s+', '_', safe.strip())
    if len(safe) > 200:
        safe = safe[:200]
    return safe + ".html"


def extract_title_from_href(href: str) -> str | None:
    if not href:
        return None
    if "/index.php/" in href:
        path = href.split("/index.php/", 1)[1].split("?")[0].split("#")[0]
        return unquote(path).replace("_", " ")
    if "title=" in href:
        match = re.search(r'title=([^&]+)', href)
        if match:
            return unquote(match.group(1)).replace("_", " ")
    return None


async def wait_for_content(tab, max_wait=30):
    """Wait until Cloudflare challenge resolves."""
    for _ in range(max_wait):
        try:
            content = await tab.get_content()
            if content and "Just a moment" not in content:
                return content
        except Exception:
            pass
        await asyncio.sleep(1)
    return None


async def get_all_pages(tab) -> list[str]:
    """Enumerate all pages via Special:AllPages."""
    titles = []
    url = f"{INDEX_URL}?title=Special:AllPages"
    page_num = 0

    while url:
        page_num += 1
        log(f"  AllPages page {page_num}... ({len(titles)} titles so far)")

        await tab.get(url)
        await asyncio.sleep(2)
        content = await wait_for_content(tab)
        if not content:
            log("    Cloudflare challenge stuck!")
            break

        soup = BeautifulSoup(content, "html.parser")

        body = soup.find("div", class_="mw-allpages-body")
        if not body:
            body = soup.find("table", class_="mw-allpages-table-chunk")

        if body:
            for link in body.find_all("a", href=True):
                title = extract_title_from_href(link["href"])
                if title and not title.startswith("Special:") and not title.startswith("User:"):
                    titles.append(title)
        else:
            log(f"    No allpages body found on page {page_num}")

        # Find "next page" link
        url = None
        nav = soup.find("div", class_="mw-allpages-nav")
        if nav:
            for link in nav.find_all("a", href=True):
                text = link.get_text().lower()
                if "next" in text:
                    url = urljoin(WIKI_BASE, link["href"])
                    break

        await asyncio.sleep(REQUEST_DELAY)

    return titles


async def download_page(tab, title: str, output_dir: Path) -> bool:
    """Download a single wiki page."""
    filename = title_to_filename(title)
    filepath = output_dir / filename

    if filepath.exists() and filepath.stat().st_size > 100:
        return True

    try:
        title_encoded = title.replace(" ", "_")

        # Use action=render for clean content
        url = f"{INDEX_URL}?title={title_encoded}&action=render"
        await tab.get(url)
        await asyncio.sleep(1)
        content = await wait_for_content(tab, max_wait=10)

        if content:
            soup = BeautifulSoup(content, "html.parser")
            body = soup.find("body")
            body_html = body.decode_contents() if body else ""

            if len(body_html) > 100:
                html = f"""<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>{title} - GECK Wiki</title>
</head>
<body>
<h1>{title}</h1>
{body_html}
</body>
</html>"""
                filepath.write_text(html, encoding="utf-8")
                return True

        # Fallback: full page, extract #mw-content-text
        url = f"{INDEX_URL}?title={title_encoded}"
        await tab.get(url)
        await asyncio.sleep(1)
        content = await wait_for_content(tab, max_wait=10)

        if content:
            soup = BeautifulSoup(content, "html.parser")
            mw_content = soup.find("div", id="mw-content-text")
            if mw_content and len(str(mw_content)) > 100:
                html = f"""<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<title>{title} - GECK Wiki</title>
</head>
<body>
<h1>{title}</h1>
{mw_content}
</body>
</html>"""
                filepath.write_text(html, encoding="utf-8")
                return True

        return False

    except Exception as e:
        log(f"    Error: {title} — {e}")
        return False


async def main(output_dir: Path):
    output_dir.mkdir(parents=True, exist_ok=True)

    log(f"Output: {output_dir}")
    log(f"Wiki:   {WIKI_BASE}")
    log("")

    # Launch undetected Chrome
    log("Step 0: Launching Chrome and solving Cloudflare challenge...")
    browser = await uc.start()
    tab = await browser.get(f"{INDEX_URL}?title=Main_Page")
    await asyncio.sleep(5)

    content = await wait_for_content(tab, max_wait=60)
    if not content:
        log("  FATAL: Could not pass Cloudflare challenge after 60s.")
        browser.stop()
        sys.exit(1)

    if "GECK" in content or "Main Page" in content:
        log("  Cloudflare challenge passed!")
    else:
        log("  WARNING: Unexpected page content")

    # Step 1: Enumerate all pages
    log("\nStep 1: Enumerating all pages...")
    titles = await get_all_pages(tab)
    titles = sorted(set(titles))
    log(f"  Found {len(titles)} unique pages")

    if not titles:
        log("FATAL: No pages found.")
        browser.stop()
        sys.exit(1)

    # Save title list for resume capability
    title_list_file = output_dir / "_page_list.txt"
    title_list_file.write_text("\n".join(titles), encoding="utf-8")
    log(f"  Saved page list to {title_list_file}")

    # Step 2: Download all pages
    log(f"\nStep 2: Downloading {len(titles)} pages...")
    downloaded = 0
    skipped = 0
    failed = 0
    failed_titles = []

    for i, title in enumerate(titles):
        filepath = output_dir / title_to_filename(title)
        if filepath.exists() and filepath.stat().st_size > 100:
            skipped += 1
            if (i + 1) % 200 == 0:
                log(f"  Progress: {i + 1}/{len(titles)} (new: {downloaded}, skipped: {skipped}, failed: {failed})")
            continue

        await asyncio.sleep(REQUEST_DELAY)
        if await download_page(tab, title, output_dir):
            downloaded += 1
        else:
            failed += 1
            failed_titles.append(title)

        if (i + 1) % 50 == 0 or i == len(titles) - 1:
            log(f"  Progress: {i + 1}/{len(titles)} (new: {downloaded}, skipped: {skipped}, failed: {failed})")

    log(f"\nDone!")
    log(f"  Downloaded: {downloaded}")
    log(f"  Skipped: {skipped}")
    log(f"  Failed: {failed}")
    log(f"  Total files: {len(list(output_dir.glob('*.html')))}")

    if failed_titles:
        failed_file = output_dir / "_failed.txt"
        failed_file.write_text("\n".join(failed_titles), encoding="utf-8")
        log(f"  Failed titles saved to {failed_file}")

    browser.stop()


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="Download GECK wiki HTML")
    parser.add_argument("--output-dir", required=True)
    args = parser.parse_args()
    uc.loop().run_until_complete(main(Path(args.output_dir)))
