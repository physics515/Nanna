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
                // Route through the daemon's single dreaming orchestrator so a
                // manual consolidation runs the *same* body as the scheduled
                // one: apply tallied feedback, flush the pending FSRS
                // testing-effect queue that every `recall` fills, then
                // consolidate. Calling `MemoryService::consolidate` directly
                // (the old shape) skipped both and left that queue to grow for
                // the daemon's whole uptime.
                //
                // Checked here — before any config/model work — because a
                // missing orchestrator is a *wiring* fault, not a runtime one:
                // failing fast keeps the precondition observable and testable
                // without needing a live LLM to reach it.
                let Some(ref dreaming) = self.dreaming else {
                    return json!({
                        "error": "dreaming_unavailable",
                        "message": "Dreaming orchestrator not configured"
                    });
                };

                // Trigger memory consolidation (requires LLM for summarization)
                let Some(ref router) = self.router else {
                    return json!({ "error": "llm_unavailable", "message": "LLM router required for consolidation" });
                };

                let router_for_summarize = router.clone();

                // Summarization models from settings, falling back to the main
                // model priority. Shared with the scheduled dream cycle so the
                // two paths cannot drift (see `crate::dream_summarizer`).
                let cfg = self.config.read().await;
                let summarize_models = crate::dream_summarizer::summarization_models(
                    &cfg.llm.summarization_priority,
                    &cfg.llm.model_priority,
                );
                let max_compression_ratio = cfg.memory.max_compression_ratio;
                let min_remaining_memories = cfg.memory.min_remaining_memories;
                drop(cfg);

                if summarize_models.is_empty() {
                    return json!({ "error": "no_models", "message": "No summarization or main models configured." });
                }

                // Size the budget to the SMALLEST window across the failover
                // list — one prompt is offered to each candidate in turn, so a
                // budget fitted to the first would overflow a smaller fallback.
                let window_tokens = crate::dream_summarizer::summarizer_context_window_tokens(
                    router,
                    &summarize_models,
                )
                .await;
                let config = ConsolidationConfig {
                    max_compression_ratio,
                    min_remaining_memories,
                    ..ConsolidationConfig::default()
                }
                .with_summarizer_context_window(window_tokens);

                info!("Consolidation summarization priority: {:?}", summarize_models);

                let summarize = crate::dream_summarizer::summarize_with_failover(
                    router_for_summarize,
                    summarize_models,
                );


                // Deliberately NOT idle-gated: the user asked for this one
                // explicitly, so it runs even while the system is busy.
                match dreaming.dream_with_consolidation(&config, summarize).await {
                    Ok(stats) => {
                        let result = stats.consolidation;
                        info!(
                            "Memory consolidation: {} processed, {} clusters, {} merged, {} expanded, {} promoted, {} demoted, {} errors",
                            result.memories_processed,
                            result.clusters_formed,
                            result.memories_merged,
                            result.memories_expanded,
                            stats.auto_promoted,
                            stats.auto_demoted,
                            result.errors.len()
                        );
                        for err in &result.errors {
                            warn!("Consolidation error: {}", err);
                        }
                        json!({
                            "status": "success",
                            "memories_processed": result.memories_processed,
                            "clusters_formed": result.clusters_formed,
                            "memories_merged": result.memories_merged,
                            "memories_deduped": result.memories_deduped,
                            "memories_expanded": result.memories_expanded,
                            "auto_promoted": stats.auto_promoted,
                            "auto_demoted": stats.auto_demoted,
                            "total_memories": stats.total_memories,
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
