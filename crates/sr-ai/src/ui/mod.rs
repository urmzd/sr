use crate::commands::commit::CommitPlan;
use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::{self, Write};
use std::time::Duration;

pub fn spinner(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

pub fn display_plan(plan: &CommitPlan) {
    println!();
    println!("═══════════════════════════════════════════════════════");
    println!("                   COMMIT PLAN");
    println!("═══════════════════════════════════════════════════════");

    for (i, commit) in plan.commits.iter().enumerate() {
        let order = commit.order.unwrap_or(i as u32 + 1);
        println!();
        println!("  [{order}] {}", commit.message);

        if let Some(body) = &commit.body
            && !body.is_empty()
        {
            println!("       {body}");
        }

        if let Some(footer) = &commit.footer
            && !footer.is_empty()
        {
            println!("       {footer}");
        }

        let fc = commit.files.len();
        let file_preview: Vec<&str> = commit.files.iter().take(5).map(|s| s.as_str()).collect();
        let suffix = if fc > 5 { "..." } else { "" };
        println!("       Files ({fc}): {}{suffix}", file_preview.join(" "));
    }

    println!();
    println!("═══════════════════════════════════════════════════════");
}

pub fn confirm(prompt: &str) -> Result<bool> {
    // Check if stdin is a TTY
    if crossterm::terminal::size().is_err() {
        return Ok(false);
    }

    print!("{prompt} ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();

    Ok(trimmed == "y" || trimmed == "yes")
}
