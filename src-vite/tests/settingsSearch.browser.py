"""Evaluate Settings tests with the project's Vue runtime and a private headless browser.

The caller supplies the already-provisioned interpreter and browser; no downloads
or user browser profiles are used. See README.md beside this file.
"""
import json
import os
import sys
from playwright.sync_api import sync_playwright

payload = json.loads(sys.stdin.buffer.read())
with sync_playwright() as playwright:
    browser = playwright.chromium.launch(
        headless=True, executable_path=os.environ['LAPCASA_TEST_BROWSER'])
    try:
        page = browser.new_page()
        page.set_content('<div id="app"></div>')
        page.add_script_tag(path=sys.argv[1])
        print(json.dumps(page.evaluate(payload['script'])))
    finally:
        try:
            if screenshot := os.environ.get('LAPCASA_TEST_SCREENSHOT'):
                page.locator('#app').screenshot(path=screenshot)
        finally:
            browser.close()
