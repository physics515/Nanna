//! Memory handlers for the [`ControlPlane`].

use super::*;

impl ControlPlane {
    // =========================================================================
    // Memory Handlers
    // =========================================================================
    
    pub(super) async fn handle_memory(&self, _client_id: &str, action: MemoryAction) -> Value {
        let Some(ref memory) = self.memory else {
            return json!({ "error": "memory_unavailable", "message": "Memory service not configured" });
        };
        
        match action {
            MemoryAction::List { scope } => {
                let all_memories = memory.list_all().await;
                let memories: Vec<_> = all_memories.into_iter()
                    .filter(|m| {
                        // Apply scope filter
                        match &scope {
                            None => true,
                            Some(s) if s == "global" => m.workspace_id.is_none(),
                            Some(ws_id) => m.workspace_id.is_none() || m.workspace_id.as_deref() == Some(ws_id),
                        }
                    })
                    .map(|m| {
                        let fact_type = m.metadata.get("fact_type")
                            .cloned()
                            .unwrap_or_else(|| "stated".to_string());
                        let created_at = chrono::DateTime::from_timestamp(m.timestamp, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| m.timestamp.to_string());

                        json!({
                            "id": m.id,
                            "content": m.content,
                            "fact_type": fact_type,
                            "importance": m.importance,
                            "state": format!("{:?}", m.state).to_lowercase(),
                            "weight": m.weight,
                            "retrievability": m.retrievability,
                            "access_count": m.access_count,
                            "created_at": created_at,
                            "session_id": m.metadata.get("session_id"),
                            "workspace_id": m.workspace_id,
                        })
                    })
                    .collect();
                json!({ "memories": memories })
            }
            MemoryAction::Search { query, limit, scope } => {
                // Use scoped recall: None = all, Some("global") = global only, Some(ws_id) = global + workspace
                let result = match &scope {
                    Some(ws_id) if ws_id != "global" => memory.recall_scoped(&query, Some(ws_id.as_str())).await,
                    Some(_) => memory.recall_scoped(&query, None).await, // "global" or None → all
                    None => memory.recall(&query).await,
                };
                match result {
                    Ok(results) => {
                        let memories: Vec<_> = results.into_iter()
                            .take(limit.unwrap_or(10))
                            .map(|r| json!({
                                "id": r.id,
                                "content": r.content,
                                "score": r.score,
                                "weight": r.weight,
                            }))
                            .collect();
                        json!({ "memories": memories, "query": query })
                    }
                    Err(e) => json!({ "error": "search_failed", "message": e.to_string() })
                }
            }
            MemoryAction::Get { id } => {
                // Get memory by ID
                if let Some(entry) = memory.get(&id).await {
                    let fact_type = entry.metadata.get("fact_type")
                        .cloned()
                        .unwrap_or_else(|| "stated".to_string());
                    let created_at = chrono::DateTime::from_timestamp(entry.timestamp, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| entry.timestamp.to_string());
                    
                    json!({
                        "memory": {
                            "id": entry.id,
                            "content": entry.content,
                            "fact_type": fact_type,
                            "importance": entry.importance,
                            "state": format!("{:?}", entry.state).to_lowercase(),
                            "weight": entry.weight,
                            "retrievability": entry.retrievability,
                            "access_count": entry.access_count,
                            "created_at": created_at,
                            "session_id": entry.metadata.get("session_id"),
                            "workspace_id": entry.workspace_id,
                        }
                    })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            MemoryAction::Create { content, tags, importance } => {
                let mut metadata = std::collections::HashMap::new();
                if let Some(tags) = tags {
                    metadata.insert("tags".to_string(), tags.join(","));
                }

                match memory.remember_with_importance(&content, metadata, importance.unwrap_or(3) as f32).await {
                    Ok((id, action)) => {
                        // Memory auto-persisted to Turso via write-through.
                        json!({
                            "id": id,
                            "action": format!("{:?}", action),
                        })
                    }
                    Err(e) => json!({ "error": "create_failed", "message": e.to_string() })
                }
            }
            MemoryAction::Update { id, content, tags: _ } => {
                // Update memory content
                if let Some(new_content) = content {
                    match memory.update_content(&id, &new_content).await {
                        Ok(()) => {
                            // Memory auto-persisted to Turso via write-through.
                            json!({ "status": "updated", "id": id })
                        }
                        Err(e) => json!({ "error": "update_failed", "message": e.to_string() })
                    }
                } else {
                    json!({ "error": "no_changes", "id": id })
                }
            }
            MemoryAction::Delete { id } => {
                match memory.forget(&id).await {
                    Ok(()) => {
                        // Memory auto-persisted to Turso via write-through.
                        json!({ "status": "deleted", "id": id })
                    }
                    Err(e) => json!({ "error": "delete_failed", "message": e.to_string() })
                }
            }
            MemoryAction::Clear => {
                memory.clear().await;
                // Note: clear() removes all in-memory entries. Individual removes
                // write-through to Turso, but bulk clear would require a separate
                // DB call. For now we log a warning.
                warn!("Memory cleared in-memory. Turso entries are NOT cleared — restart will reload them.");
                info!("Cleared all memories (in-memory only)");
                json!({ "status": "cleared" })
            }
            MemoryAction::Stats => {
                let stats = memory.stats().await;
                json!({
                    "total": stats.total,
                    "active": stats.active,
                    "dormant": stats.dormant,
                    "silent": stats.silent,
                    "unavailable": stats.unavailable,
                })
            }
            MemoryAction::Consolidate => {
                // Trigger memory consolidation (requires LLM for summarization)
                let Some(ref router) = self.router else {
                    return json!({ "error": "llm_unavailable", "message": "LLM router required for consolidation" });
                };

                let router_for_summarize = router.clone();

                // Use the summarization model priority from settings
                let cfg = self.config.read().await;
                let mut summarize_models = cfg.llm.summarization_priority.clone();
                // Fall back to main model priority if no summarization models configured
                if summarize_models.is_empty() {
                    summarize_models = cfg.llm.model_priority.clone();
                }
                let max_compression_ratio = cfg.memory.max_compression_ratio;
                let min_remaining_memories = cfg.memory.min_remaining_memories;
                drop(cfg);

                if summarize_models.is_empty() {
                    return json!({ "error": "no_models", "message": "No summarization or main models configured." });
                }

                // Resolve the actual summarizer before deriving consolidation limits.
                let summarizer_info = router.get_model_info(&summarize_models[0]).await;
                let config = ConsolidationConfig {
                    max_compression_ratio,
                    min_remaining_memories,
                    ..ConsolidationConfig::default()
                }
                .with_summarizer_context_window(summarizer_info.hard_input_limit());

                info!("Consolidation summarization priority: {:?}", summarize_models);

                let summarize = move |prompt: String| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>> {
                    let router = router_for_summarize.clone();
                    let models = summarize_models.clone();
                    Box::pin(async move {
                        let mut last_err = String::from("No summarization models configured");
                        for model in &models {
                            let request = nanna_llm::CompletionRequest::default()
                                .with_model(model)
                                .with_message(nanna_llm::Message::user(&prompt));
                            match router.complete(model, request).await {
                                Ok(result) => return Ok(result),
                                Err(e) => {
                                    tracing::warn!("Summarization model {} failed: {}", model, e);
                                    last_err = format!("{}: {}", model, e);
                                }
                            }
                        }
                        Err(format!("All summarization models failed. Last error: {}", last_err))
                    })
                };
                
                // P13 unification: dream through the shared `DreamingService`
                // when one is attached, so a user-triggered consolidation runs
                // the full multi-phase cycle — pending feedback applied, the
                // testing-effect FSRS flush, the min-memories floor — and not
                // just its final clustering phase. Deliberately `dream_with`
                // (ungated) rather than `dream_if_idle_with`: the user asked for
                // this one, so the idle gate must not veto it. Falls back to the
                // low-level call when memory is on but no orchestrator is
                // attached (minimal constructions), so this can never regress.
                let consolidation = match self.dreaming {
                    Some(ref dreaming) => dreaming
                        .dream_with(&config, summarize)
                        .await
                        .map(|stats| stats.consolidation),
                    None => memory.consolidate(&config, summarize).await,
                };

                match consolidation {
                    Ok(result) => {
                        info!("Memory consolidation: {} processed, {} clusters, {} merged, {} expanded, {} errors",
                              result.memories_processed, result.clusters_formed,
                              result.memories_merged, result.memories_expanded,
                              result.errors.len());
                        for err in &result.errors {
                            warn!("Consolidation error: {}", err);
                        }
                        json!({
                            "status": "success",
                            "memories_processed": result.memories_processed,
                            "clusters_formed": result.clusters_formed,
                            "memories_merged": result.memories_merged,
                            "memories_expanded": result.memories_expanded,
                            "errors": result.errors,
                        })
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        error!("Memory consolidation failed: {}", err_msg);
                        json!({ "error": "consolidation_failed", "message": err_msg })
                    }
                }
            }
        }
    }
}
