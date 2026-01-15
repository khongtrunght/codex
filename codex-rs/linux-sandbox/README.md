# zenith-linux-sandbox

This crate is responsible for producing:

- a `zenith-linux-sandbox` standalone executable for Linux that is bundled with the Node.js version of the Zenith CLI
- a lib crate that exposes the business logic of the executable as `run_main()` so that
  - the `zenith-exec` CLI can check if its arg0 is `zenith-linux-sandbox` and, if so, execute as if it were `zenith-linux-sandbox`
  - this should also be true of the `zenith` multitool CLI
