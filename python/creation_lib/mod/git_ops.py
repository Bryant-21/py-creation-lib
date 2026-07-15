"""Git operations for mod repos — commit, pull, push, checkout, Gitea init.

All functions accept explicit Path arguments.
"""
from __future__ import annotations

import json
import logging
import subprocess
import urllib.error
import urllib.request
from datetime import datetime
from pathlib import Path
from typing import Callable
from urllib.parse import urlparse, urlunparse

_log = logging.getLogger(__name__)


def _run_git(mod_dir: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess:
    """Run a git command in mod_dir."""
    cmd = ["git"] + list(args)
    return subprocess.run(
        cmd, cwd=str(mod_dir), capture_output=True, text=True, check=check,
    )


def _ensure_git_repo(mod_dir: Path) -> None:
    """Validate mod_dir exists and is a git repo."""
    if not mod_dir.is_dir():
        raise FileNotFoundError(f"Mod directory not found: {mod_dir}")
    if not (mod_dir / ".git").is_dir():
        raise RuntimeError(f"{mod_dir.name} is not a git repository.")


def _has_local_changes(mod_dir: Path) -> bool:
    """Check if working tree has uncommitted changes."""
    r = _run_git(mod_dir, "status", "--porcelain", check=False)
    return bool(r.stdout.strip())


def _current_branch(mod_dir: Path) -> str:
    """Get current branch name."""
    r = _run_git(mod_dir, "symbolic-ref", "--quiet", "--short", "HEAD", check=False)
    if r.returncode == 0 and r.stdout.strip():
        return r.stdout.strip()
    r = _run_git(mod_dir, "rev-parse", "--abbrev-ref", "HEAD", check=False)
    return r.stdout.strip() or "main"


def _has_upstream(mod_dir: Path) -> bool:
    """Check if current branch tracks an upstream."""
    r = _run_git(mod_dir, "rev-parse", "--verify", "@{upstream}", check=False)
    return r.returncode == 0


def _authed_remote(mod_dir: Path, gitea_user: str, gitea_token: str) -> str:
    """Build a one-shot authenticated URL from the origin remote.

    Used as the push/pull target so credentials never touch git config
    or credential helpers.  e.g. https://user:token@host/repo.git
    """
    r = _run_git(mod_dir, "remote", "get-url", "origin", check=False)
    url = r.stdout.strip()
    if not url:
        return ""
    parsed = urlparse(url)
    authed = parsed._replace(
        netloc=f"{gitea_user}:{gitea_token}@{parsed.hostname}"
        + (f":{parsed.port}" if parsed.port else ""),
    )
    return urlunparse(authed)


def _push(
    mod_dir: Path, branch: str, *,
    gitea_user: str = "", gitea_token: str = "", set_upstream: bool = False,
) -> subprocess.CompletedProcess:
    """Push to remote. Uses authed URL when credentials are provided."""
    if gitea_user and gitea_token:
        target = _authed_remote(mod_dir, gitea_user, gitea_token)
        if set_upstream:
            # push to authed URL, then set upstream tracking to clean origin
            r = _run_git(mod_dir, "-c", "http.sslVerify=false",
                         "push", target, branch, check=False)
            if r.returncode == 0:
                _run_git(mod_dir, "branch", f"--set-upstream-to=origin/{branch}", branch,
                         check=False)
            return r
        return _run_git(mod_dir, "-c", "http.sslVerify=false",
                        "push", target, branch, check=False)
    # No credentials — passthrough to let terminal/GCM handle it
    if set_upstream:
        return _run_git(mod_dir, "push", "-u", "origin", branch, check=False)
    return _run_git(mod_dir, "push", check=False)


def _pull(
    mod_dir: Path, *, gitea_user: str = "", gitea_token: str = "",
) -> subprocess.CompletedProcess:
    """Pull from remote. Uses authed URL when credentials are provided."""
    if gitea_user and gitea_token:
        target = _authed_remote(mod_dir, gitea_user, gitea_token)
        branch = _current_branch(mod_dir)
        return _run_git(mod_dir, "-c", "http.sslVerify=false",
                        "pull", target, branch, "--ff-only", check=False)
    return _run_git(mod_dir, "pull", "--ff-only", check=False)


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------

def git_commit(
    mod_dir: Path, mod_name: str, *, gitea_user: str = "", gitea_token: str = "",
) -> str:
    """Stage all, commit with timestamp, push. Returns commit hash or empty string."""
    _ensure_git_repo(mod_dir)
    _run_git(mod_dir, "config", "http.sslVerify", "false", check=False)

    if not _has_local_changes(mod_dir):
        _log.info("No local changes to commit.")
        return ""

    _run_git(mod_dir, "add", "-A")

    # Check if staging produced anything
    r = _run_git(mod_dir, "diff", "--cached", "--quiet", "--ignore-submodules", "--", check=False)
    if r.returncode == 0:
        _log.info("No staged changes to commit.")
        return ""

    timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    msg = f"Update {mod_name} via Mod Builder on {timestamp}"
    _run_git(mod_dir, "commit", "-m", msg)

    branch = _current_branch(mod_dir)
    has_up = _has_upstream(mod_dir)
    _push(mod_dir, branch, gitea_user=gitea_user, gitea_token=gitea_token,
          set_upstream=not has_up)

    # Return commit hash
    r = _run_git(mod_dir, "rev-parse", "HEAD", check=False)
    return r.stdout.strip()


def git_pull(
    mod_dir: Path, *, gitea_user: str = "", gitea_token: str = "",
) -> None:
    """Pull --ff-only from origin."""
    _ensure_git_repo(mod_dir)
    _run_git(mod_dir, "config", "http.sslVerify", "false", check=False)

    if _has_local_changes(mod_dir):
        raise RuntimeError("Local changes detected. Commit or checkout changes before pulling.")

    r = _run_git(mod_dir, "remote", "get-url", "origin", check=False)
    if r.returncode != 0:
        raise RuntimeError(f"No 'origin' remote configured for {mod_dir.name}")

    r = _pull(mod_dir, gitea_user=gitea_user, gitea_token=gitea_token)
    if r.returncode != 0:
        raise RuntimeError(f"Pull failed: {r.stderr.strip()}")


def git_push(
    mod_dir: Path, *, gitea_user: str = "", gitea_token: str = "",
) -> None:
    """Push to tracked upstream, or set upstream to origin."""
    _ensure_git_repo(mod_dir)
    _run_git(mod_dir, "config", "http.sslVerify", "false", check=False)

    branch = _current_branch(mod_dir)
    has_up = _has_upstream(mod_dir)
    r = _push(mod_dir, branch, gitea_user=gitea_user, gitea_token=gitea_token,
              set_upstream=not has_up)
    if r.returncode != 0:
        raise RuntimeError(f"Push failed: {r.stderr.strip()}")


def git_checkout(mod_dir: Path) -> None:
    """Hard reset + clean untracked files."""
    _ensure_git_repo(mod_dir)
    _run_git(mod_dir, "reset", "--hard", "HEAD")
    _run_git(mod_dir, "clean", "-fd")


# ---------------------------------------------------------------------------
# Gitea repo initialization
# ---------------------------------------------------------------------------

_GITIGNORE = """\
# Build artifacts (rebuilt from source YAML + assets)
*.esp
*.esl
*.esm
*.ba2
*.bsa
*.pex
release/

# Temp / intermediate files
*.assbin
*.tmp
*_deploy*
*_import*

# Autosave files (Substance Painter, etc.)
*_autosave_*.spp

# xmake / Visual Studio build artifacts (xse plugins)
build/
build_*/
.xmake/
vsxmake*/
.vs/
.vscode/
.idea/
.cache/
compile_commands.json
*.sln
*.vcxproj*
*.pdb
*.exp
*.lib

# xse plugin install-staging output (regenerated by `xmake install` from src/ + web/)
F4SE/
SKSE/
SFSE/
NVSE/
FOSE/
PrismaUI_F4/
"""

_GITATTRIBUTES = """\
# 3D source files
*.fbx filter=lfs diff=lfs merge=lfs -text
*.3ds filter=lfs diff=lfs merge=lfs -text
*.obj filter=lfs diff=lfs merge=lfs -text
*.max filter=lfs diff=lfs merge=lfs -text
*.blend filter=lfs diff=lfs merge=lfs -text
*.nif filter=lfs diff=lfs merge=lfs -text

# Textures
*.dds filter=lfs diff=lfs merge=lfs -text
*.psd filter=lfs diff=lfs merge=lfs -text
*.spp filter=lfs diff=lfs merge=lfs -text

# Game meshes and behaviors
*.nif filter=lfs diff=lfs merge=lfs -text
*.hkx filter=lfs diff=lfs merge=lfs -text

# Audio
*.wav filter=lfs diff=lfs merge=lfs -text
*.fuz filter=lfs diff=lfs merge=lfs -text
*.xwm filter=lfs diff=lfs merge=lfs -text
*.lip filter=lfs diff=lfs merge=lfs -text
"""


def gitea_init(
    mod_dir: Path,
    mod_name: str,
    *,
    game: str = "",
    gitea_url: str = "",
    gitea_user: str = "",
    gitea_org: str = "",
    gitea_token: str = "",
    on_progress: Callable[[str], None] | None = None,
) -> None:
    """Initialize git repo with .gitignore/.gitattributes and push to Gitea.

    Uses Gitea API first (if token is set), falls back to push-to-create.
    """
    def _emit(msg: str, *, level: int = logging.INFO) -> None:
        _log.log(level, msg)
        if on_progress:
            on_progress(msg)

    if not gitea_url or not gitea_user:
        _emit("GITEA_URL and GITEA_USER must be set — skipping git init.", level=logging.WARNING)
        return

    if (mod_dir / ".git").is_dir():
        _emit(f"{mod_dir} is already a git repo — skipping init.")
        return

    # Determine game label for repo description
    game_labels = {
        "fo4": "Fallout 4 mod",
        "skyrimse": "Skyrim SE mod",
        "starfield": "Starfield mod",
    }
    game_label = game_labels.get(game, f"{game} mod" if game else "Mod")

    # Init repo
    _emit("Initializing local git repository...")
    _run_git(mod_dir, "init", "-q")

    # Write .gitignore and .gitattributes
    (mod_dir / ".gitignore").write_text(_GITIGNORE, encoding="utf-8")
    (mod_dir / ".gitattributes").write_text(_GITATTRIBUTES, encoding="utf-8")

    # Set remote — plain URL (credentials never stored in config)
    repo_owner = gitea_org or gitea_user
    base_url = gitea_url.rstrip("/")
    remote_url = f"{base_url}/{repo_owner}/{mod_name}.git"
    _run_git(mod_dir, "remote", "add", "origin", remote_url)
    _run_git(mod_dir, "config", "http.sslVerify", "false")

    # Speed up initial add: increase pack-objects threads and LFS batch size
    _run_git(mod_dir, "config", "pack.threads", "0", check=False)        # auto thread count
    _run_git(mod_dir, "config", "lfs.concurrenttransfers", "8", check=False)
    _run_git(mod_dir, "config", "lfs.batch", "true", check=False)

    # Initial commit
    _emit("Creating initial commit...")
    _run_git(mod_dir, "add", "-A")
    _run_git(mod_dir, "commit", "-q", "-m", f"Initial mod structure for {mod_name}")

    # Get current branch
    branch = _current_branch(mod_dir)

    # Try Gitea API first (if token is set)
    if gitea_token:
        _emit("Creating remote Gitea repository...")
        if gitea_org:
            api_url = f"{gitea_url.rstrip('/')}/api/v1/orgs/{gitea_org}/repos"
        else:
            api_url = f"{gitea_url.rstrip('/')}/api/v1/user/repos"

        _log.debug("Gitea API URL: %s", api_url)

        payload = json.dumps({
            "name": mod_name,
            "description": f"{game_label}: {mod_name}",
            "private": True,
        }).encode("utf-8")

        req = urllib.request.Request(
            api_url,
            data=payload,
            headers={
                "Authorization": f"token {gitea_token}",
                "Content-Type": "application/json",
            },
            method="POST",
        )

        try:
            import ssl
            ctx = ssl.create_default_context()
            ctx.check_hostname = False
            ctx.verify_mode = ssl.CERT_NONE
            resp = urllib.request.urlopen(req, context=ctx)
            status = resp.getcode()
        except urllib.error.HTTPError as e:
            status = e.code
            body = ""
            try:
                body = e.read().decode("utf-8", errors="replace")
            except Exception:
                pass
            _emit(f"API repo creation HTTP {status}: {body}", level=logging.WARNING)
        except Exception as e:
            _emit(f"API repo creation failed: {e}", level=logging.WARNING)
            status = 0

        if status in (201, 409):  # 409 = already exists
            _emit("Pushing initial commit to Gitea...")
            r = _push(mod_dir, branch, gitea_user=gitea_user, gitea_token=gitea_token,
                       set_upstream=True)
            if r.returncode == 0:
                _emit("Git repo created via API and pushed.")
                return
            _emit(
                f"Push failed after API create (rc={r.returncode}): {(r.stderr or r.stdout).strip()}",
                level=logging.WARNING,
            )
        else:
            _emit(
                f"API repo creation failed (HTTP {status}) — trying push-to-create...",
                level=logging.WARNING,
            )

    # Fallback: push-to-create
    _emit("Pushing initial commit to Gitea...")
    r = _push(mod_dir, branch, gitea_user=gitea_user, gitea_token=gitea_token,
              set_upstream=True)
    if r.returncode == 0:
        _emit("Git repo created and pushed (push-to-create).")
        return
    if not gitea_token:
        _emit(
            "Push failed (rc=%d). No GITEA_TOKEN set for API fallback. "
            "Repo initialized locally — push manually when ready." % r.returncode,
            level=logging.WARNING,
        )


def gitea_delete_repo(
    mod_dir: Path,
    *,
    gitea_token: str = "",
) -> None:
    """Delete the remote Gitea repository for this mod.

    Reads the remote URL from ``git remote get-url origin`` to determine
    owner and repo name, then issues DELETE /api/v1/repos/{owner}/{repo}.
    """
    if not gitea_token:
        raise RuntimeError("No Gitea token configured — cannot delete remote repository.")

    r = _run_git(mod_dir, "remote", "get-url", "origin", check=False)
    if r.returncode != 0 or not r.stdout.strip():
        raise RuntimeError("No git remote 'origin' found — cannot delete remote repository.")

    remote_url = r.stdout.strip()
    parsed = urlparse(remote_url)
    clean = parsed._replace(netloc=parsed.hostname + (f":{parsed.port}" if parsed.port else ""))
    base_url = urlunparse(clean._replace(path=""))

    path_parts = parsed.path.strip("/").split("/")
    if len(path_parts) < 2:
        raise RuntimeError(f"Cannot parse owner/repo from remote URL: {remote_url}")
    owner = path_parts[0]
    repo_name = path_parts[1].removesuffix(".git")

    api_url = f"{base_url}/api/v1/repos/{owner}/{repo_name}"
    _log.info("Deleting remote Gitea repo: %s/%s", owner, repo_name)

    req = urllib.request.Request(
        api_url,
        headers={"Authorization": f"token {gitea_token}"},
        method="DELETE",
    )
    try:
        import ssl
        ctx = ssl.create_default_context()
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
        resp = urllib.request.urlopen(req, context=ctx)
        status = resp.getcode()
    except urllib.error.HTTPError as e:
        status = e.code
        if status == 404:
            _log.warning("Remote repo not found (already deleted?).")
            return
        body = ""
        try:
            body = e.read().decode("utf-8", errors="replace")
        except Exception:
            pass
        raise RuntimeError(f"Gitea API DELETE failed (HTTP {status}): {body}") from e
    except Exception as e:
        raise RuntimeError(f"Gitea API DELETE failed: {e}") from e

    if status == 204:
        _log.info("Remote Gitea repository deleted.")
    else:
        _log.warning("Unexpected response deleting remote repo (HTTP %d).", status)
