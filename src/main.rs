//! rival-council: a swarm of reviewer agents that argue with each other.
//!
//! Round 1  three rivals read the same material and write a brief, in parallel, blind.
//! Round 2  each rival receives the other two briefs and writes a rebuttal (peer cross-talk).
//! Round 3  the adjudicator receives all six documents and rules.
//!
//! Every runtime event is appended to out/runs.jsonl in the orbit monitor's schema
//! ({ts, from, to, kind, goal, label}) so monitor/orbits-monitor.html can show the swarm live.

use autoagents::async_trait;
use autoagents::core::actor::Topic;
use autoagents::core::agent::memory::SlidingWindowMemory;
use autoagents::core::agent::prebuilt::executor::BasicAgent;
use autoagents::core::agent::task::Task;
use autoagents::core::agent::{ActorAgent, AgentBuilder, AgentHooks, Context};
use autoagents::core::environment::Environment;
use autoagents::core::runtime::{SingleThreadedRuntime, TypedRuntime};
use autoagents::core::utils::BoxEventStream;
use autoagents::llm::builder::LLMBuilder;
use autoagents::llm::LLMProvider;
use autoagents::protocol::Event;
use autoagents_derive::{agent, AgentHooks};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use tokio_stream::StreamExt;

// ---------------------------------------------------------------- the agents

#[agent(
    name = "policy_analyst",
    description = "You are a policy analyst who has briefed cabinet-level officials. You judge a lecture by one test: could a public-policy graduate student restate each slide's point in one sentence, with no equation, to a skeptical CFO? You argue for fewer numbers, one question per slide, and converted units (dollars and days, never tokens). You write in plain prose, no bullet lists, under 450 words. Never use em dashes."
)]
#[derive(AgentHooks)]
pub struct PolicyAnalyst {}

#[agent(
    name = "instructional_designer",
    description = "You are an instructional designer who builds graduate courses. You judge a lecture by scaffolding: does each slide connect to something the students already met in the syllabus, and does the whole thing leave exactly three takeaways a student could write down? You argue for sequencing, named roles, and a close that maps to the course's own vocabulary (financial, governance, risk). You write in plain prose, no bullet lists, under 450 words. Never use em dashes."
)]
#[derive(AgentHooks)]
pub struct InstructionalDesigner {}

#[agent(
    name = "student",
    description = "You are a second-year public-policy graduate student taking this class. You have no engineering background. You judge a lecture by what you would actually write in your notes and what you would be too embarrassed to ask about. Be concrete: name the slide, say what you'd write down, say where you'd get lost. Plain prose, no bullet lists, under 400 words. Never use em dashes."
)]
#[derive(AgentHooks)]
pub struct Student {}

#[agent(
    name = "adjudicator",
    description = "You are a former agency CFO and chief risk officer who now teaches this course. You receive three rival briefs and three rebuttals about a guest lecture. Rule on them. State which rival's theory of accessibility governs and why, list the specific changes to the deck that survive the debate (slide number, change, which rival earned it), list what was argued and rejected, and name the one risk that remains. Be decisive and short, under 600 words. No bullet lists except the change list. Never use em dashes."
)]
pub struct Adjudicator {
    out_dir: String,
}

#[async_trait]
impl AgentHooks for Adjudicator {
    async fn on_run_complete(&self, _task: &Task, result: &Self::Output, _ctx: &Context) {
        let path = format!("{}/ruling.md", self.out_dir);
        let _ = fs::write(&path, result.as_str());
        println!("\n==== RULING ====\n{result}\n==== written to {path} ====");
    }
}

// ---------------------------------------------------------------- the event log

struct Log {
    file: Mutex<fs::File>,
}

impl Log {
    fn new(path: &str) -> anyhow::Result<Self> {
        let file = fs::OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { file: Mutex::new(file) })
    }
    fn emit(&self, from: &str, to: &str, kind: &str, goal: &str, label: &str) {
        let line = serde_json::json!({
            "ts": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            "from": from, "to": to, "kind": kind, "goal": goal,
            "label": label.chars().take(140).collect::<String>(),
            "indirect": false
        });
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{line}");
        }
    }
}

// ---------------------------------------------------------------- the swarm

const RIVALS: [&str; 3] = ["policy_analyst", "instructional_designer", "student"];

#[derive(Default)]
struct State {
    round1: HashMap<String, String>,
    round2: HashMap<String, String>,
}

fn build_llm() -> anyhow::Result<Arc<dyn LLMProvider>> {
    // Anthropic by default. Set OPENAI_API_KEY (and optionally OPENAI_BASE_URL) to use an
    // OpenAI-compatible endpoint instead, including a local one.
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        use autoagents::llm::backends::anthropic::Anthropic;
        let model = std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-sonnet-5-5".into());
        let llm = LLMBuilder::<Anthropic>::new()
            .api_key(key)
            .model(&model)
            .max_tokens(1500)
            .temperature(0.4)
            .build()?;
        println!("model: anthropic/{model}");
        return Ok(llm);
    }
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        use autoagents::llm::backends::openai::{OpenAI, OpenAIApiMode};
        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".into());
        // chat/completions is what local and third-party OpenAI-compatible servers speak
        let mut b = LLMBuilder::<OpenAI>::new().api_key(key).model(&model).max_tokens(1500).temperature(0.4)
            .api_mode(OpenAIApiMode::ChatCompletions);
        if let Ok(url) = std::env::var("OPENAI_BASE_URL") {
            b = b.base_url(url);
        }
        println!("model: openai-compatible/{model}");
        return Ok(b.build()?);
    }
    anyhow::bail!("set ANTHROPIC_API_KEY (or OPENAI_API_KEY) before running")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    autoagents::init_logging();
    let material_path = std::env::args().nth(1).unwrap_or_else(|| "input/material.md".into());
    let material = fs::read_to_string(&material_path)
        .map_err(|e| anyhow::anyhow!("cannot read {material_path}: {e}"))?;
    fs::create_dir_all("out")?;
    let log = Arc::new(Log::new("out/runs.jsonl")?);
    let llm = build_llm()?;

    let runtime = SingleThreadedRuntime::new(None);
    let memory = Box::new(SlidingWindowMemory::new(8));

    // One topic per agent, so the coordinator (and later, the rivals' rebuttal round) can address each one.
    let topics: HashMap<&str, Topic<Task>> =
        RIVALS.iter().map(|r| (*r, Topic::<Task>::new(*r))).collect();
    let adjudicate = Topic::<Task>::new("adjudicator");

    AgentBuilder::<_, ActorAgent>::new(BasicAgent::new(PolicyAnalyst {}))
        .llm(llm.clone()).runtime(runtime.clone()).subscribe(topics["policy_analyst"].clone())
        .memory(memory.clone()).build().await?;
    AgentBuilder::<_, ActorAgent>::new(BasicAgent::new(InstructionalDesigner {}))
        .llm(llm.clone()).runtime(runtime.clone()).subscribe(topics["instructional_designer"].clone())
        .memory(memory.clone()).build().await?;
    AgentBuilder::<_, ActorAgent>::new(BasicAgent::new(Student {}))
        .llm(llm.clone()).runtime(runtime.clone()).subscribe(topics["student"].clone())
        .memory(memory.clone()).build().await?;
    AgentBuilder::<_, ActorAgent>::new(BasicAgent::new(Adjudicator { out_dir: "out".into() }))
        .llm(llm.clone()).runtime(runtime.clone()).subscribe(adjudicate.clone())
        .memory(memory.clone()).build().await?;

    let mut environment = Environment::new(None);
    environment.register_runtime(runtime.clone()).await?;
    let receiver = environment.take_event_receiver(None).await?;

    let done = Arc::new(Notify::new());
    drive(receiver, runtime.clone(), log.clone(), done.clone());

    // Round 1: the same material to all three rivals, in parallel, blind.
    log.emit("dave", "coordinator", "task", "round1", "material handed to the council");
    let prompt = format!(
        "ROUND 1. Read the material below and write your brief on what would make this lecture more accessible to the class. Argue your own theory; you have not seen anyone else's.\n\n{material}"
    );
    for r in RIVALS {
        log.emit("coordinator", r, "task", "round1", "round 1 brief: the material, blind");
        runtime.publish(&topics[r], Task::new(prompt.clone())).await?;
    }

    environment.run()?;

    tokio::select! {
        _ = done.notified() => {
            println!("council finished");
            if let Err(e) = environment.shutdown().await { eprintln!("shutdown: {e}"); }
        }
        res = environment.wait() => { println!("environment ended: {res:?}"); }
        _ = tokio::signal::ctrl_c() => {
            println!("ctrl-c, shutting down");
            if let Err(e) = environment.shutdown().await { eprintln!("shutdown: {e}"); }
        }
    }
    Ok(())
}

/// The coordinator loop: watches runtime events, logs them, and opens the next round when a round completes.
fn drive(mut events: BoxEventStream<Event>, runtime: Arc<SingleThreadedRuntime>, log: Arc<Log>, done: Arc<Notify>) {
    tokio::spawn(async move {
        let mut st = State::default();
        while let Some(ev) = events.next().await {
            match ev {
                Event::TaskStarted { actor_name, task_description, .. } => {
                    let round = round_of(&task_description);
                    log.emit(&actor_name, &actor_name, "write", round, "started thinking");
                }
                Event::ToolCallRequested { actor_id, tool_name, .. } => {
                    log.emit(&actor_id.to_string(), "tools", "write", "tools", &format!("tool call: {tool_name}"));
                }
                Event::TaskError { actor_id, error, .. } => {
                    eprintln!("agent {actor_id} failed: {error}");
                    log.emit(&actor_id.to_string(), "coordinator", "result", "error", &format!("error: {error}"));
                }
                Event::TaskComplete { actor_name, result, .. } => {
                    // BasicAgent results arrive as JSON: {"response": "...", "done": true}
                    let text = serde_json::from_str::<serde_json::Value>(&result).ok()
                        .and_then(|v| v.get("response").and_then(|r| r.as_str()).map(String::from))
                        .unwrap_or_else(|| result.clone());
                    let text = text.trim().to_string();
                    if actor_name == "adjudicator" {
                        log.emit("adjudicator", "files", "result", "round3", "ruling written");
                        log.emit("coordinator", "dave", "result", "round3", "ruling ready for Dave");
                        done.notify_one();
                        continue;
                    }
                    if !st.round1.contains_key(&actor_name) {
                        // ---- a round-1 brief came in
                        let path = format!("out/{actor_name}-brief.md");
                        let _ = fs::write(&path, &text);
                        println!("[round 1] {actor_name} brief in ({} words)", text.split_whitespace().count());
                        log.emit(&actor_name, "files", "result", "round1", &format!("brief written: {}", first_line(&text)));
                        st.round1.insert(actor_name.clone(), text);
                        if st.round1.len() == RIVALS.len() {
                            // ---- open round 2: each rival gets the other two briefs (the cross-talk)
                            println!("[round 2] all briefs in; rivals now read each other");
                            for r in RIVALS {
                                let others: Vec<String> = RIVALS.iter().filter(|o| **o != r)
                                    .map(|o| format!("### Brief from {o}\n{}", st.round1[*o])).collect();
                                for o in RIVALS.iter().filter(|o| **o != r) {
                                    log.emit(o, r, "read", "round2", &format!("{o} brief sent to {r}"));
                                }
                                let prompt = format!(
                                    "ROUND 2. You have now read the other two briefs, below. Write your rebuttal: where they are wrong, where they changed your mind, and the single change to the deck you would fight for. Under 350 words, plain prose.\n\n{}",
                                    others.join("\n\n")
                                );
                                if let Err(e) = runtime.publish(&Topic::<Task>::new(r), Task::new(prompt)).await {
                                    eprintln!("publish round 2 to {r}: {e}");
                                }
                            }
                        }
                    } else if !st.round2.contains_key(&actor_name) {
                        // ---- a round-2 rebuttal came in
                        let path = format!("out/{actor_name}-rebuttal.md");
                        let _ = fs::write(&path, &text);
                        println!("[round 2] {actor_name} rebuttal in");
                        log.emit(&actor_name, "files", "result", "round2", &format!("rebuttal written: {}", first_line(&text)));
                        st.round2.insert(actor_name.clone(), text);
                        if st.round2.len() == RIVALS.len() {
                            // ---- open round 3: the adjudicator gets everything
                            println!("[round 3] all rebuttals in; adjudicator rules");
                            let mut dossier = String::from("ROUND 3. Rule on the debate below.\n");
                            for r in RIVALS {
                                dossier.push_str(&format!("\n## {r}: brief\n{}\n\n## {r}: rebuttal\n{}\n", st.round1[r], st.round2[r]));
                                log.emit(r, "adjudicator", "read", "round3", &format!("{r} brief and rebuttal to the adjudicator"));
                            }
                            log.emit("coordinator", "adjudicator", "task", "round3", "rule on the debate");
                            if let Err(e) = runtime.publish(&Topic::<Task>::new("adjudicator"), Task::new(dossier)).await {
                                eprintln!("publish round 3: {e}");
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    });
}

fn round_of(desc: &str) -> &'static str {
    if desc.starts_with("ROUND 1") { "round1" } else if desc.starts_with("ROUND 2") { "round2" } else { "round3" }
}

fn first_line(s: &str) -> String {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(90).collect()
}
