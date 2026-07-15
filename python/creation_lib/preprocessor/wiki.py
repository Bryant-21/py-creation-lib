"""Build {game}_wiki.db from wiki HTML files (Papyrus + Creation Kit).

Merges the old preprocess_wiki.py (Papyrus) and preprocess_ck.py (CK) into a
single script that outputs one database with a 'category' column
("papyrus" or "creation_kit").
"""

import glob
import json
import os
import sys
import html2text
from bs4 import BeautifulSoup, Tag

from creation_lib.db.native_runtime import BulkInserter

LANGUAGE_REF_PAGES = {
    "Array_Reference", "Expression_Reference", "Function_Reference",
    "Operator_Reference", "Property_Reference", "Statement_Reference",
    "Variable_Reference", "Events_Reference", "Flag_Reference",
    "Keyword_Reference", "Identifier_Reference", "Cast_Reference",
    "Literals_Reference", "Default_Value_Reference", "Group_Reference",
    "Script_File_Structure", "Language_Reference_Notation",
    "Extending_Scripts_(Papyrus)", "Arrays_(Papyrus)",
}

OVERVIEW_PAGES = {
    "Papyrus_Introduction", "Papyrus_Overview", "Papyrus_FAQs",
    "Papyrus_Tutorials", "Papyrus_Glossary",
}


def is_papyrus_page(stem):
    return (
        stem.endswith("_Script")
        or stem.endswith("_(Papyrus)")
        or ".(Papyrus)" in stem
        or "_-_" in stem
        or stem in LANGUAGE_REF_PAGES
        or stem in OVERVIEW_PAGES
    )


def get_papyrus_subcategory(stem):
    if stem.endswith("_Script"):
        return "script_api"
    if "_-_" in stem:
        return "function"
    if stem.endswith("_(Papyrus)") or ".(Papyrus)" in stem:
        return "function"
    if stem in LANGUAGE_REF_PAGES:
        return "language_ref"
    if stem in OVERVIEW_PAGES:
        return "overview"
    return "other"


RECORD_TYPE_NAMES = {
    "Actor", "Activator", "ActorValueInfo", "Ammo", "Apparatus", "Armor",
    "ArmorAddon", "Book", "Cell", "Climate", "Container", "ConstructibleObject",
    "CombatStyle", "Door", "EffectSetting", "Enchantment", "EncounterZone",
    "EquipSlot", "Explosion", "Faction", "Flora", "Furniture", "GlobalVariable",
    "Grass", "Hazard", "HeadPart", "IdleMarker", "ImageSpace", "Ingredient",
    "Key", "Keyword", "LeveledActor", "LeveledItem", "LeveledSpell", "Light",
    "Location", "LocationAlias", "LocationRefType", "MagicEffect", "Message",
    "MiscItem", "MovableStatic", "NPC", "NPC_", "Outfit", "Package", "Perk",
    "PlacedObject", "Projectile", "Quest", "Race", "ReferenceAlias", "Scene",
    "Scroll", "Shout", "SoulGem", "Sound", "SoundCategory", "SoundDescriptor",
    "Spell", "Static", "TalkingActivator", "TextureSet", "Tree", "Weapon",
    "Weather", "Worldspace",
}

TOOL_STEMS = {
    "3ds_Max", "Blender", "FO4Edit", "F4SE", "NifTools", "Archive2",
    "NifSkope", "GECK", "LOOT", "Wrye_Bash",
}

EDITOR_UI_SUFFIXES = ("_Tab", "_Window", "_Button", "_Dialog", "_Menu", "_Panel")


def get_ck_subcategory(stem):
    if stem.startswith("Bethesda_Tutorial_"):
        return "tutorial"
    if "_ini" in stem.lower() or "Prefs" in stem:
        return "configuration"
    if stem.endswith(EDITOR_UI_SUFFIXES):
        return "editor_ui"
    if stem.startswith("Category__"):
        return "category_page"
    for tool in TOOL_STEMS:
        if stem.startswith(tool) or stem == tool:
            return "tools"
    if stem.startswith("Adobe_"):
        return "tools"
    for rt in RECORD_TYPE_NAMES:
        if stem == rt or stem.startswith(rt + "_") or stem.startswith(rt + "-"):
            return "record_type"
    return "other"


def extract_extends(soup):
    for b in soup.find_all("b"):
        if b.get_text(strip=True) == "Extends:":
            a = b.find_next("a")
            if a and a.get("href", "").endswith("_Script.html"):
                return a["href"].replace("_Script.html", "")
    return ""


def extract_function_sections(soup):
    syntax = return_type = examples_text = ""
    for h2 in soup.find_all("h2"):
        headline = h2.find("span", class_="mw-headline")
        if not headline:
            continue
        section_id = headline.get("id", "")
        siblings = []
        for sib in h2.next_siblings:
            if getattr(sib, "name", None) == "h2":
                break
            siblings.append(sib)

        if section_id == "Syntax":
            for el in siblings:
                if isinstance(el, Tag):
                    pre = el.find("pre", class_="de1")
                    if pre:
                        syntax = pre.get_text(" ", strip=True)
                        break
        elif section_id == "Return_Value":
            for el in siblings:
                if isinstance(el, Tag):
                    t = el.get_text(" ", strip=True)
                    if t:
                        return_type = t
                        break
        elif section_id == "Examples":
            parts = []
            for el in siblings:
                if isinstance(el, Tag):
                    pre = el.find("pre", class_="de1")
                    if pre:
                        parts.append(pre.get_text(" ", strip=True))
            examples_text = "\n".join(parts)

    return syntax, return_type, examples_text


def extract_content(html_path):
    with open(html_path, encoding="utf-8", errors="replace") as f:
        raw = f.read()

    soup = BeautifulSoup(raw, "html.parser")

    h1 = soup.find("h1", id="firstHeading")
    if h1:
        title = h1.get_text(strip=True)
    else:
        title_tag = soup.find("title")
        title = title_tag.get_text(strip=True) if title_tag else os.path.basename(html_path)

    wiki_cats = []
    catlinks = soup.find(id="catlinks")
    if catlinks:
        for a in catlinks.find_all("a"):
            text = a.get_text(strip=True)
            if text and text.lower() not in ("categories", "category"):
                wiki_cats.append(text)

    for tag in soup.find_all(id=["toc", "siteSub", "contentSub", "catlinks"]):
        tag.decompose()
    for tag in soup.find_all(class_=["navbox", "toc", "printfooter"]):
        tag.decompose()

    content_div = soup.find(id="mw-content-text") or soup.find(id="bodyContent")
    if content_div:
        content_html = str(content_div)
    else:
        content_html = str(soup.body) if soup.body else raw

    converter = html2text.HTML2Text()
    converter.ignore_links = False
    converter.ignore_images = True
    converter.body_width = 0
    content_text = converter.handle(content_html)

    return title, content_text.strip(), ", ".join(wiki_cats), soup


PAGE_COLS = ["filename", "title", "category", "wiki_category", "content"]

BULK_SCHEMA = json.dumps({
    "tables": [
        {"name": "pages", "pk": "filename", "on_conflict": "IGNORE", "columns": PAGE_COLS},
    ],
})

CREATE_TABLES_DDL = """
CREATE TABLE IF NOT EXISTS pages (
    filename TEXT PRIMARY KEY,
    title TEXT,
    category TEXT,
    wiki_category TEXT,
    content TEXT
);
CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
    title, category, content,
    content=pages, content_rowid=rowid,
    tokenize='unicode61'
);
"""

CREATE_INDEXES_SQL = "CREATE INDEX IF NOT EXISTS idx_pages_category ON pages(category);"


def build_db(wiki_dir, db_path, build_embeddings=False):
    all_html = glob.glob(os.path.join(wiki_dir, "*.html"))
    if not all_html:
        all_html = glob.glob(os.path.join(wiki_dir, "**", "*.html"), recursive=True)

    papyrus_count = 0
    ck_count = 0
    embed_texts: list[str] = []
    embed_ids: list[str] = []

    os.makedirs(os.path.dirname(db_path), exist_ok=True)

    with BulkInserter(db_path, BULK_SCHEMA, fresh=True) as bulk:
        bulk.execute(CREATE_TABLES_DDL)
        buf = {c: [] for c in PAGE_COLS}

        for path in all_html:
            fname = os.path.basename(path)
            stem = fname[:-5]

            if stem.startswith("Special__"):
                continue

            if is_papyrus_page(stem):
                category = "papyrus"
                wiki_category = get_papyrus_subcategory(stem)
            else:
                category = "creation_kit"
                wiki_category = get_ck_subcategory(stem)

            try:
                title, content, _wiki_cats, soup = extract_content(path)
            except Exception as e:
                print(f"  SKIP {fname}: {e}")
                continue

            buf["filename"].append(stem)
            buf["title"].append(title)
            buf["category"].append(category)
            buf["wiki_category"].append(wiki_category)
            buf["content"].append(content)

            if category == "papyrus":
                papyrus_count += 1
            else:
                ck_count += 1

            embed_texts.append(f"{title} {category} {wiki_category} {content[:500]}")
            embed_ids.append(stem)

            if len(buf["filename"]) >= 500:
                bulk.add_chunk("pages", buf)
                for lst in buf.values():
                    lst.clear()

        if buf["filename"]:
            bulk.add_chunk("pages", buf)

        bulk.create_indexes(CREATE_INDEXES_SQL)
        bulk.rebuild_fts("pages_fts")

    if build_embeddings and embed_texts:
        from creation_lib.db.embeddings import build_vec_index
        print(f"\nBuilding sqlite-vec index ({len(embed_texts)} documents)...")
        build_vec_index(embed_texts, embed_ids, db_path)

    return papyrus_count, ck_count


def main():
    import argparse

    parser = argparse.ArgumentParser(description="Build wiki search database")
    parser.add_argument("--game", default="fo4")
    parser.add_argument("--wiki-dir", required=True)
    parser.add_argument("--db-path", required=True)
    parser.add_argument("--embeddings", action="store_true")
    args = parser.parse_args()
    game = args.game

    from creation_lib.core.game_profiles import get_profile, GAME_PROFILES

    if game not in GAME_PROFILES:
        print(f"ERROR: Invalid --game '{game}'. Valid: {', '.join(sorted(GAME_PROFILES))}")
        sys.exit(1)
    profile = get_profile(game)
    if not profile.wiki_dir:
        print(f"No wiki available for '{game}' (wiki_dir is None)")
        sys.exit(0)

    wiki_dir = args.wiki_dir
    db_path = args.db_path
    build_embeddings = args.embeddings

    if not os.path.isdir(wiki_dir):
        print(f"ERROR: Wiki directory not found: {wiki_dir}")
        sys.exit(1)

    print(f"Building wiki database [{game}]")
    print(f"  Wiki dir : {wiki_dir}")
    print(f"  Database : {db_path}")
    print()

    papyrus_count, ck_count = build_db(wiki_dir, db_path, build_embeddings=build_embeddings)

    print(f"\nDatabase built at: {db_path}")
    print(f"  papyrus     : {papyrus_count}")
    print(f"  creation_kit: {ck_count}")
    print(f"  TOTAL       : {papyrus_count + ck_count}")


if __name__ == "__main__":
    main()
