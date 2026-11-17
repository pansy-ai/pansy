//! `pansy plugin`: magpie's community plugins, added, switched, signed in
//! to and brought over from magpie.

use std::io::{BufRead, Write};

use anyhow::{Result, anyhow, bail};
use pansy_core::plugin::Listing;
use pansy_plugins::{host, import, market, middleware, signin, store};
use serde_json::{Value, json};

use crate::print_json;

/// Runs `fut` on the plugins' runtime.
fn run<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    pansy_plugins::block(fut)
}

pub fn command(words: &[&str], json: bool) -> Result<()> {
    match words {
        [] | ["list"] => list(json),
        ["discover"] => discover(json),
        ["search", words @ ..] if !words.is_empty() => search(&words.join(" "), json),
        ["add", spec] => {
            let spec = spec.to_string();
            let added = run(async move { store::add(&spec).await })?;
            println!(
                "Added {} ({}).",
                added.kinds.name,
                added.kinds.words().join(", ")
            );
            if added.kinds.in_host() {
                match run(host::list_now()) {
                    Ok(listing) => {
                        let mine: Vec<String> = listing
                            .loaded
                            .iter()
                            .filter(|l| l.spec == added.entry.spec)
                            .flat_map(|l| {
                                l.error
                                    .iter()
                                    .map(|e| format!("it didn't load: {e}"))
                                    .chain(l.provides.iter().map(|p| {
                                        format!("it signs in to {p}: pansy plugin login {p}")
                                    }))
                            })
                            .collect();
                        for line in mine {
                            println!("  {line}");
                        }
                    }
                    Err(e) => eprintln!("pansy: the plugins couldn't start: {e:#}"),
                }
            }
            if added.kinds.agent.is_some() {
                let kept = run(pansy_plugins::agents::reload())?;
                for a in kept.agents.iter().filter(|a| a.spec == added.entry.spec) {
                    println!(
                        "  it adds the agent {}: pansy connect {} <provider/model>",
                        a.name, a.id
                    );
                }
            }
            Ok(())
        }
        ["rm" | "remove", word] => {
            let word = word.to_string();
            let gone = run(async move { store::remove(&word).await })?;
            let _ = run(host::list_now());
            println!("Removed {}.", gone.package());
            Ok(())
        }
        ["on", word] | ["off", word] => {
            let off = words[0] == "off";
            let e = store::set_off(word, off)?;
            let _ = run(host::list_now());
            println!("{} is {}.", e.package(), if off { "off" } else { "on" });
            Ok(())
        }
        ["update"] | ["update", _] => {
            let word = words.get(1).map(|w| w.to_string());
            let done = run(async move { store::update_plugins(word.as_deref()).await })?;
            let _ = run(host::list_now());
            if done.is_empty() {
                println!(
                    "Nothing to update: plugins added by path are loaded from where they are."
                );
            }
            for d in done {
                println!("Updated {d}.");
            }
            Ok(())
        }
        ["options", word] => {
            let list = store::List::load()?;
            let at = list
                .find(word)
                .ok_or_else(|| anyhow!("no plugin {word} was added"))?;
            let e = &list.plugins[at];
            let example = e.kinds().options;
            print_json(&json!({"options": e.options_value(), "example": example}))
        }
        ["options", word, value] => {
            let value: Value =
                serde_json::from_str(value).map_err(|e| anyhow!("the options aren't JSON: {e}"))?;
            store::set_options(word, value)?;
            println!("Saved {word}'s options.");
            Ok(())
        }
        ["use", word, provider] => {
            store::use_for(word, provider)?;
            let _ = run(host::list_now());
            println!("{word} serves {provider} now.");
            Ok(())
        }
        ["refresh"] => {
            let listing = run(host::list_now())?;
            println!(
                "{} providers from {} plugins.",
                listing.providers.len(),
                listing.loaded.len()
            );
            Ok(())
        }
        ["login", provider] => login(provider, None, json),
        ["login", provider, method] => login(provider, Some(method.parse()?), json),
        ["logout", provider] | ["logout", provider, _] => {
            let provider = provider.to_string();
            let account = words.get(2).map(|a| a.to_string());
            run(async move { host::sign_out(&provider, account.as_deref()).await })?;
            println!("Signed out.");
            Ok(())
        }
        ["import", "magpie"] => {
            let done = run(import::from_magpie())?;
            if json {
                return print_json(&done);
            }
            for a in &done.added {
                println!("Added {a}.");
            }
            for (spec, e) in &done.failed {
                eprintln!("pansy: {spec}: {e}");
            }
            println!(
                "{} plugins added, {} were here already; {} sign-ins copied.",
                done.added.len(),
                done.kept.len(),
                done.accounts
            );
            Ok(())
        }
        _ => bail!(
            "usage: pansy plugin [list|discover|search|add|rm|on|off|update|options|use|refresh|login|logout|import magpie]; pansy help"
        ),
    }
}

fn list(json: bool) -> Result<()> {
    let installed = store::installed()?;
    let listing = Listing::load();
    let agents = pansy_plugins::agents::cached();
    if json {
        let rows: Vec<Value> = installed
            .iter()
            .map(|i| {
                json!({
                    "spec": i.entry.spec,
                    "package": i.entry.package(),
                    "off": i.entry.off,
                    "kinds": i.kinds.words(),
                    "version": i.kinds.version,
                    "error": listing.loaded.iter().find(|l| l.spec == i.entry.spec).and_then(|l| l.error.clone()),
                    "providers": listing.providers.iter().filter(|p| p.spec == i.entry.spec).collect::<Vec<_>>(),
                })
            })
            .collect();
        return print_json(&rows);
    }
    if installed.is_empty() {
        println!("No plugins yet: pansy plugin discover, then pansy plugin add <name>");
        return Ok(());
    }
    let states = if installed
        .iter()
        .any(|i| !i.entry.off && i.kinds.middleware.is_some())
    {
        run(middleware::states()).unwrap_or_default()
    } else {
        Value::Null
    };
    for i in &installed {
        let version = if i.kinds.version.is_empty() {
            String::new()
        } else {
            format!(" {}", i.kinds.version)
        };
        let off = if i.entry.off { " — off" } else { "" };
        println!(
            "{}{version} ({}){off}",
            i.entry.package(),
            i.kinds.words().join(", ")
        );
        if let Some(e) = listing
            .loaded
            .iter()
            .find(|l| l.spec == i.entry.spec)
            .and_then(|l| l.error.as_ref())
        {
            println!("  didn't load: {e}");
        }
        for p in listing.providers.iter().filter(|p| p.spec == i.entry.spec) {
            let who: Vec<String> = p
                .accounts
                .iter()
                .map(|a| {
                    if a.account_id.is_empty() {
                        a.key.clone()
                    } else {
                        a.account_id.clone()
                    }
                })
                .collect();
            let signed = if who.is_empty() {
                format!("not signed in: pansy plugin login {}", p.id)
            } else {
                format!("signed in as {}", who.join(", "))
            };
            println!(
                "  {} ({}): {} models, {signed}",
                p.name,
                p.id,
                p.models.len()
            );
        }
        if i.kinds.middleware.is_some() {
            let name = middleware::name_of(i);
            if let Some(s) = states.get(&name) {
                println!(
                    "  middleware {name}: {} — {} calls, {} failed{}",
                    s["hooks"]
                        .as_array()
                        .map(|h| h
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", "))
                        .unwrap_or_default(),
                    s["calls"],
                    s["failures"],
                    s["lastError"]
                        .as_str()
                        .filter(|e| !e.is_empty())
                        .map(|e| format!("; last: {e}"))
                        .unwrap_or_default()
                );
            }
        }
        for a in agents.agents.iter().filter(|a| a.spec == i.entry.spec) {
            println!("  adds the agent {} ({})", a.name, a.id);
        }
    }
    Ok(())
}

/// The community plugins, by shelf as magpie's market has them: OpenCode
/// plugins, pi packages, middleware, agents; most downloaded first.
fn discover(json: bool) -> Result<()> {
    let mut list = run(market::all(false));
    list.sort_by(|a, b| b.weekly.cmp(&a.weekly).then_with(|| a.name.cmp(&b.name)));
    if json {
        return print_json(&list);
    }
    let shelves = [
        ("opencode", "OpenCode plugins"),
        ("pi", "pi packages"),
        ("middleware", "Gateway middleware"),
        ("agent", "Agents"),
        ("tools", "Tools (pansy doesn't run these yet)"),
        ("page", "Pages (pansy doesn't run these yet)"),
    ];
    for (shelf, title) in shelves {
        let of: Vec<&market::Listed> = list.iter().filter(|l| l.shelf == shelf).collect();
        if !of.is_empty() {
            println!("\n{title} ({})", of.len());
            print_listed(&of);
        }
    }
    Ok(())
}

/// npm's OpenCode plugins and pi packages for `q`.
fn search(q: &str, json: bool) -> Result<()> {
    let q = q.to_owned();
    let list = run(async move { market::search(&q).await })?;
    if json {
        return print_json(&list);
    }
    if list.is_empty() {
        println!("Nothing on npm.");
    }
    print_listed(&list.iter().collect::<Vec<_>>());
    Ok(())
}

fn print_listed(list: &[&market::Listed]) {
    let zh = std::env::var("LANG").is_ok_and(|l| l.starts_with("zh"));
    for l in list {
        let summary = l
            .summary
            .get(if zh { "zh" } else { "en" })
            .or_else(|| l.summary.get("en"))
            .cloned()
            .unwrap_or_default();
        let by = if l.community {
            "magpie community".to_owned()
        } else {
            l.publisher.clone()
        };
        let weekly = if l.weekly > 0 {
            format!(", {}/week", l.weekly)
        } else {
            String::new()
        };
        println!("  {} — {} ({by}{weekly})", l.name, l.package);
        if !summary.is_empty() {
            println!("    {summary}");
        }
    }
}

fn ask(question: &str) -> Result<String> {
    eprint!("{question}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

fn login(provider: &str, method: Option<usize>, json: bool) -> Result<()> {
    let methods = signin::methods(provider)?;
    if methods.is_empty() {
        bail!("{provider} has no way to sign in");
    }
    let method = match method {
        Some(m) => m,
        None if methods.len() == 1 => 0,
        None => {
            for (i, m) in methods.iter().enumerate() {
                eprintln!("  {i}. {} ({})", m.label, m.kind);
            }
            ask("Sign in how? ")?
                .parse()
                .map_err(|_| anyhow!("a method's number"))?
        }
    };
    let p = provider.to_owned();
    let mut flow = run(async move { signin::start(&p, method).await })?;
    loop {
        match flow.state.as_str() {
            "asking" => {
                let prompt = flow.prompt.clone().unwrap_or(Value::Null);
                if !flow.invalid.is_empty() {
                    eprintln!("  {}", flow.invalid);
                }
                if let Some(options) = prompt["options"].as_array() {
                    for (i, o) in options.iter().enumerate() {
                        eprintln!("  {i}. {}", o["label"].as_str().unwrap_or_default());
                    }
                }
                let answer = ask(&format!("{} ", prompt["message"].as_str().unwrap_or("?")))?;
                let answer = match prompt["options"].as_array() {
                    Some(options) => answer
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| options.get(i))
                        .and_then(|o| o["value"].as_str())
                        .map(str::to_owned)
                        .unwrap_or(answer),
                    None => answer,
                };
                let id = flow.id.clone();
                flow = run(async move { signin::answer(&id, &answer).await })?;
            }
            "key" => {
                if !flow.invalid.is_empty() {
                    eprintln!("  {}", flow.invalid);
                }
                let key = ask("Key: ")?;
                let id = flow.id.clone();
                flow = run(async move { signin::answer(&id, &key).await })?;
            }
            "code" => {
                eprintln!(
                    "Open this page, then paste the code it gives:\n\n  {}\n",
                    flow.url
                );
                if !flow.instructions.is_empty() {
                    eprintln!("{}", flow.instructions);
                }
                let code = ask("Code: ")?;
                let id = flow.id.clone();
                flow = run(async move { signin::answer(&id, &code).await })?;
            }
            "waiting" => {
                if !flow.opened {
                    eprintln!("Open this page to sign in:\n\n  {}\n", flow.url);
                }
                if !flow.instructions.is_empty() {
                    eprintln!("{}", flow.instructions);
                }
                eprintln!("Waiting for the sign-in… (Ctrl-C to stop)");
                while flow.state == "waiting" {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    flow =
                        signin::status(&flow.id).ok_or_else(|| anyhow!("the sign-in is gone"))?;
                }
            }
            "done" => {
                if json {
                    return print_json(&flow);
                }
                println!("Signed in ({}).", flow.account);
                if let Some(p) = Listing::load().providers.iter().find(|p| p.id == provider) {
                    println!(
                        "{} has {} models: {provider}/<model> (pansy models lists them).",
                        p.name,
                        p.models.len()
                    );
                }
                return Ok(());
            }
            "canceled" => bail!("the sign-in was stopped"),
            _ => bail!("{}", flow.error),
        }
    }
}
