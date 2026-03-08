"""Tests for the rex_py Renderer."""

import os
import pytest
from rex_py import Renderer

FIXTURES_ROOT = os.path.join(
    os.path.dirname(
        os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    ),
    "fixtures",
    "basic",
)


@pytest.fixture(scope="module")
def rex():
    """Shared Renderer instance for all tests in this module."""
    r = Renderer(root=FIXTURES_ROOT)
    yield r
    r.close()


def test_init(rex):
    assert len(rex.pages) > 0
    assert rex.build_id
    assert rex.client_dir
    assert os.path.isdir(rex.client_dir)


def test_render_by_module_name(rex):
    html = rex.render("index", props={"message": "Hello from Python!", "timestamp": 0})
    assert "<!DOCTYPE html>" in html
    assert "<h1>Rex!</h1>" in html
    assert "Hello from Python!" in html


def test_render_by_url_path(rex):
    html = rex.render("/about")
    assert "<!DOCTYPE html>" in html
    assert "About" in html


def test_render_with_no_props(rex):
    html = rex.render("about")
    assert "<!DOCTYPE html>" in html


def test_render_dynamic_route(rex):
    html = rex.render(
        "blog/[slug]", props={"slug": "hello", "title": "Test", "content": "Body"}
    )
    assert "<!DOCTYPE html>" in html


def test_render_includes_client_scripts(rex):
    html = rex.render("index", props={"message": "test", "timestamp": 0})
    assert '<script type="module"' in html
    assert "/_rex/static/" in html


def test_render_includes_css(rex):
    html = rex.render("index", props={"message": "test", "timestamp": 0})
    # Global CSS from _app should be inlined
    assert "<style>" in html


def test_manifest(rex):
    manifest = rex.manifest
    assert isinstance(manifest, dict)
    assert "build_id" in manifest
    assert "pages" in manifest
    assert "/" in manifest["pages"]


def test_pages_list(rex):
    pages = sorted(rex.pages)
    assert "index" in pages
    assert "about" in pages
    assert "blog/[slug]" in pages


def test_repr(rex):
    r = repr(rex)
    assert "Renderer(" in r
    assert "pages=" in r
    assert "build_id=" in r


def test_closed_renderer():
    r = Renderer(root=FIXTURES_ROOT)
    r.close()
    with pytest.raises(RuntimeError, match="closed"):
        r.render("index")


def test_invalid_module_name(rex):
    """Rendering a non-existent module should raise an error from V8."""
    with pytest.raises(RuntimeError):
        rex.render("nonexistent_page")


def test_invalid_url_path(rex):
    """Rendering a non-matching URL path should raise."""
    with pytest.raises(RuntimeError, match="No route matches"):
        rex.render("/this/path/does/not/exist")
