use anyhow::{Result, anyhow, ensure};
use clap::{Args, Subcommand};

use crate::api_client::{HttpAgentApi, api_error};

#[derive(Args, Debug, Clone)]
pub(crate) struct SkillsArgs {
    #[command(subcommand)]
    command: SkillsCommand,
}

#[derive(Subcommand, Debug, Clone)]
enum SkillsCommand {
    /// List skills available to a session.
    List(SkillsListArgs),
    /// Ask the agent to read and use a skill, steering the current run if one exists.
    Use(SkillsUseArgs),
}

#[derive(Args, Debug, Clone)]
struct SkillsListArgs {
    /// JSON-RPC agent API URL.
    #[arg(long = "api-url", env = "LIGHTSPEED_API_URL")]
    api_url: String,
    /// Emit the skill list as JSON.
    #[arg(long)]
    json: bool,
    /// Session id to inspect.
    #[arg(long)]
    session: String,
}

#[derive(Args, Debug, Clone)]
struct SkillsUseArgs {
    /// JSON-RPC agent API URL.
    #[arg(long = "api-url", env = "LIGHTSPEED_API_URL")]
    api_url: String,
    /// Emit the ordinary run-start or steering response as JSON.
    #[arg(long)]
    json: bool,
    /// Session in which to use the skill.
    #[arg(long)]
    session: String,
    /// Skill id from the session catalog.
    skill_id: String,
}

pub(crate) async fn handle(args: SkillsArgs) -> Result<()> {
    match args.command {
        SkillsCommand::List(args) => list(args).await,
        SkillsCommand::Use(args) => use_skill(args).await,
    }
}

async fn list(args: SkillsListArgs) -> Result<()> {
    let response = HttpAgentApi::new(args.api_url)
        .list_skills(api::SkillListParams {
            session_id: args.session,
        })
        .await
        .map_err(api_error)?
        .result;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }
    println!(
        "catalogRef {}",
        response.catalog_ref.as_deref().unwrap_or("-")
    );
    println!("skills {}", response.skills.len());
    for skill in &response.skills {
        let enabled = if skill.enabled { "enabled" } else { "disabled" };
        println!("{} {} {}", skill.skill_id, enabled, skill.name);
        println!("  {}", skill.description);
        let api::SkillLocationView::Vfs { skill_doc_path, .. } = &skill.location;
        println!("  VFS {skill_doc_path}");
    }
    Ok(())
}

async fn use_skill(args: SkillsUseArgs) -> Result<()> {
    let api = HttpAgentApi::new(args.api_url);
    let catalog = api
        .list_skills(api::SkillListParams {
            session_id: args.session.clone(),
        })
        .await
        .map_err(api_error)?
        .result;
    let text = skill_selection_input(&catalog, &args.skill_id)?;
    let session = api
        .read_session(api::SessionReadParams {
            session_id: args.session.clone(),
            run_limit: Some(0),
        })
        .await
        .map_err(api_error)?
        .result
        .session;
    let items = vec![api::InputItem::Text { origin: None, text }];
    if let Some(run) = session.active_run {
        let response = api
            .steer_run(api::RunSteerParams {
                session_id: args.session,
                run_id: run.id,
                items,
            })
            .await
            .map_err(api_error)?
            .result;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&response)?);
        } else {
            println!("steered {} to use {}", response.run.id, args.skill_id);
        }
    } else {
        let response = api
            .start_run(api::RunStartParams {
                session_id: args.session,
                source: api::RunStartSource::Input { items },
                submission_id: None,
                config: None,
                notify_on_terminal: None,
            })
            .await
            .map_err(api_error)?
            .result;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&response)?);
        } else {
            println!("started {} to use {}", response.run.id, args.skill_id);
        }
    }
    Ok(())
}

pub(crate) fn skill_selection_input(
    catalog: &api::SkillListResponse,
    skill_id: &str,
) -> Result<String> {
    let skill = catalog
        .skills
        .iter()
        .find(|skill| skill.skill_id == skill_id)
        .ok_or_else(|| anyhow!("skill {skill_id:?} is not in the session catalog"))?;
    ensure!(skill.enabled, "skill {skill_id:?} is disabled");
    let api::SkillLocationView::Vfs {
        skill_dir_path,
        skill_doc_path,
    } = &skill.location;
    Ok(format!(
        "Use the {} skill ({}). Read its SKILL.md through the VFS file tool at {} before following its instructions. Resolve supporting files relative to the VFS directory {}.",
        serde_json::to_string(&skill.name)?,
        serde_json::to_string(&skill.skill_id)?,
        serde_json::to_string(skill_doc_path)?,
        serde_json::to_string(skill_dir_path)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> api::SkillListResponse {
        api::SkillListResponse {
            catalog_ref: Some("sha256:catalog".into()),
            skills: vec![api::SkillListItem {
                skill_id: "skill:review".into(),
                name: "Review".into(),
                description: "Review changes".into(),
                short_description: None,
                enabled: true,
                location: api::SkillLocationView::Vfs {
                    skill_dir_path: "/library/review notes".into(),
                    skill_doc_path: "/library/review notes/SKILL.md".into(),
                },
            }],
        }
    }

    #[test]
    fn selection_instructs_an_ordinary_read_with_source_and_base_directory() {
        assert_eq!(
            skill_selection_input(&catalog(), "skill:review").unwrap(),
            "Use the \"Review\" skill (\"skill:review\"). Read its SKILL.md through the VFS file tool at \"/library/review notes/SKILL.md\" before following its instructions. Resolve supporting files relative to the VFS directory \"/library/review notes\"."
        );
    }

    #[test]
    fn selection_rejects_missing_and_disabled_skills() {
        let mut catalog = catalog();
        assert!(skill_selection_input(&catalog, "missing").is_err());
        catalog.skills[0].enabled = false;
        assert!(skill_selection_input(&catalog, "skill:review").is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn use_submits_normal_input_when_idle_and_steers_the_current_run() {
        use serde_json::json;
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        use tokio::net::TcpListener;

        for status in [None, Some("running"), Some("parked")] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let active_run = status.map(|status| {
                    json!({
                        "id": "run_1", "status": status, "acceptedAtMs": 1,
                        "source": {"type": "input"},
                    })
                });
                let responses = [
                    serde_json::to_value(catalog()).unwrap(),
                    json!({"session": {
                        "id": "session_1", "status": if status.is_some() { "active" } else { "idle" },
                        "retention": {"rootSessionId": "session_1"}, "managed": false,
                        "configRevision": 0, "createdAtMs": 1, "updatedAtMs": 1,
                        "activeContext": {"revision": 0}, "activeRun": active_run,
                        "runs": [{"id": "run_2", "status": "queued", "acceptedAtMs": 2, "source": {"type": "input"}}],
                    }, "hasOlderRuns": false}),
                    json!({"steeringId": "steer_1", "run": {
                        "id": if status.is_some() { "run_1" } else { "run_3" },
                        "status": status.unwrap_or("queued"), "source": {"type": "input", "items": []},
                    }}),
                ];
                let mut requests = Vec::new();
                for result in responses {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut stream = BufReader::new(stream);
                    let mut content_length = None;
                    loop {
                        let mut line = String::new();
                        assert!(stream.read_line(&mut line).await.unwrap() > 0);
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                content_length = Some(value.trim().parse::<usize>().unwrap());
                            }
                        }
                    }
                    let mut body = vec![0; content_length.expect("JSON body length")];
                    stream.read_exact(&mut body).await.unwrap();
                    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let body = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": request["id"], "result": {"result": result}})).unwrap();
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream
                        .get_mut()
                        .write_all(headers.as_bytes())
                        .await
                        .unwrap();
                    stream.get_mut().write_all(&body).await.unwrap();
                    requests.push(request);
                }
                requests
            });
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                use_skill(SkillsUseArgs {
                    api_url: format!("http://{address}/rpc"),
                    json: true,
                    session: "session_1".into(),
                    skill_id: "skill:review".into(),
                }),
            )
            .await;
            if !matches!(&result, Ok(Ok(()))) {
                server.abort();
            }
            result
                .expect("selection request timeout")
                .expect("select skill");
            let requests = server.await.unwrap();
            assert_eq!(requests[0]["method"], api::METHOD_SESSION_SKILLS_LIST);
            assert_eq!(requests[1]["method"], api::METHOD_SESSION_READ);
            let submission = &requests[2];
            assert_eq!(submission["params"]["sessionId"], "session_1");
            let items = if status.is_some() {
                assert_eq!(submission["method"], api::METHOD_SESSION_RUNS_STEER);
                assert_eq!(submission["params"]["runId"], "run_1");
                &submission["params"]["items"]
            } else {
                assert_eq!(submission["method"], api::METHOD_SESSION_RUNS_START);
                assert_eq!(submission["params"]["source"]["type"], "input");
                &submission["params"]["source"]["items"]
            };
            assert_eq!(items[0]["type"], "text");
            assert!(
                items[0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("/library/review notes/SKILL.md")
            );
            assert_eq!(items.as_array().unwrap().len(), 1);
        }
    }
}
