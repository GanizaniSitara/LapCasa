# Frontend tests

Run `node --test --test-concurrency=1 tests/*.test.mjs` from `src-vite`, with the project's provisioned
Node dependencies. Browser tests also need an existing Python environment with
Playwright (tested with 1.58.0) and a Chromium-based browser.

Set `LAPCASA_TEST_PYTHON` to that environment's interpreter and
`LAPCASA_TEST_BROWSER` to the browser executable. The tests launch a private,
headless instance and close it after each case. They do not download browsers,
install packages, or read an existing browser profile.

For one browser case, `LAPCASA_TEST_SCREENSHOT` optionally saves its final `#app` crop.
Serial test files avoid running the portability guard's nested browser suite alongside a second copy.

The Settings driver and assertions live here in the repository. No workstation
scripts directory or fixed interpreter location is required.
