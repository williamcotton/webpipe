### Getting Started

- **Run a file**:
  - `cargo run example.wp`
- **Run tests inside a .wp**:
  - `cargo run example.wp --test`
- **Release build**:
  - `cargo build --release` → `target/release/webpipe`
- **Docker**: see `deployment-docker.md`.

Environment variables are loaded from `.env` and `.env.local` in the same directory as your `.wp` file when the server starts.


