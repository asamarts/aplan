mod harness;
mod paper;

fn main() -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if matches!(
        args.first().map(String::as_str),
        Some("review" | "review-self-test" | "review-export" | "review-audit")
    ) {
        paper::command(&args, &root)
    } else {
        harness::command(&args, &root)
    }
}
