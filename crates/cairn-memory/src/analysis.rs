//! Architecture report generator and activity heatmap for the memory graph dashboard.

use crate::{MemoryGraph, MemoryGraphEdge, MemoryGraphNode};
use cairn_core::Memory;
use std::collections::{HashMap, HashSet, VecDeque};

/// A generated architecture report.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ArchitectureReport {
    pub project: String,
    pub file_count: usize,
    pub edge_count: usize,
    pub community_count: usize,
    pub god_nodes: Vec<GodNodeEntry>,
    pub bridges: Vec<BridgeEntry>,
    pub cycles: Vec<Vec<String>>,
    pub isolation_ratio: f64,
    pub markdown: String,
    pub language_breakdown: HashMap<String, usize>,
    pub surprising_connections: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GodNodeEntry {
    pub name: String,
    pub edge_count: usize,
    pub kind: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BridgeEntry {
    pub name: String,
    pub centrality: f64,
    pub kind: String,
}

/// Generate a full architecture report from the memory graph.
pub fn generate_architecture_report(graph: &MemoryGraph) -> ArchitectureReport {
    let file_count = graph.nodes.len();
    let edge_count = graph.edges.len();

    let communities = detect_communities(&graph.nodes, &graph.edges);
    let community_count = communities.len();

    let god_nodes = find_god_nodes(&graph.nodes, &graph.edges, 10);
    let bridges_list = compute_bridge_centrality(&graph.edges);
    let mut bridges: Vec<BridgeEntry> = bridges_list
        .into_iter()
        .take(10)
        .map(|(name, centrality)| {
            let kind = graph
                .nodes
                .iter()
                .find(|n| n.id == name)
                .map(|n| n.kind.clone())
                .unwrap_or_default();
            BridgeEntry {
                name,
                centrality,
                kind,
            }
        })
        .collect();
    bridges.sort_by(|a, b| {
        b.centrality
            .partial_cmp(&a.centrality)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let cycles = find_cycles(&graph.edges);

    let singleton_count = communities.iter().filter(|c| c.len() == 1).count();
    let isolation_ratio = if file_count == 0 {
        0.0
    } else {
        singleton_count as f64 / file_count as f64
    };

    let language_breakdown = build_language_breakdown(&graph.nodes);

    let surprising_connections = find_surprising_connections(graph);

    let markdown = render_markdown(
        file_count,
        edge_count,
        community_count,
        &god_nodes,
        &bridges,
        &cycles,
        isolation_ratio,
        &language_breakdown,
        &surprising_connections,
    );

    ArchitectureReport {
        project: "memory".to_string(),
        file_count,
        edge_count,
        community_count,
        god_nodes,
        bridges,
        cycles,
        isolation_ratio,
        markdown,
        language_breakdown,
        surprising_connections,
    }
}

/// Detect communities via BFS connected components.
pub fn detect_communities(
    nodes: &[MemoryGraphNode],
    edges: &[MemoryGraphEdge],
) -> Vec<Vec<String>> {
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for node in nodes {
        adj.entry(node.id.as_str()).or_default();
    }
    for edge in edges {
        adj.entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
        adj.entry(edge.target.as_str())
            .or_default()
            .push(edge.source.as_str());
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut communities: Vec<Vec<String>> = Vec::new();

    for node in nodes {
        if visited.contains(node.id.as_str()) {
            continue;
        }
        let mut community: Vec<String> = Vec::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        queue.push_back(&node.id);
        visited.insert(&node.id);

        while let Some(current) = queue.pop_front() {
            community.push(current.to_string());
            if let Some(neighbors) = adj.get(current) {
                for neighbor in neighbors {
                    if visited.insert(neighbor) {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
        communities.push(community);
    }

    communities
}

/// Find god-nodes (degree > 3, sorted by edge count).
pub fn find_god_nodes(
    nodes: &[MemoryGraphNode],
    edges: &[MemoryGraphEdge],
    top_n: usize,
) -> Vec<GodNodeEntry> {
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for node in nodes {
        degree.insert(&node.id, 0);
    }
    for edge in edges {
        *degree.entry(edge.source.as_str()).or_insert(0) += 1;
        *degree.entry(edge.target.as_str()).or_insert(0) += 1;
    }

    let mut entries: Vec<GodNodeEntry> = degree
        .into_iter()
        .filter(|(_, count)| *count > 3)
        .map(|(name, edge_count)| {
            let kind = nodes
                .iter()
                .find(|n| n.id == name)
                .map(|n| n.kind.clone())
                .unwrap_or_default();
            GodNodeEntry {
                name: name.to_string(),
                edge_count,
                kind,
            }
        })
        .collect();

    entries.sort_by_key(|e| std::cmp::Reverse(e.edge_count));
    entries.truncate(top_n);
    entries
}

/// Simplistic cycle detection: DFS with back-edge check. Only finds cycles of length >= 3.
pub fn find_cycles(edges: &[MemoryGraphEdge]) -> Vec<Vec<String>> {
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        adj.entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
        adj.entry(edge.target.as_str())
            .or_default()
            .push(edge.source.as_str());
    }

    let mut cycles: Vec<Vec<String>> = Vec::new();
    let mut visited: HashSet<&str> = HashSet::new();
    let mut parent: HashMap<&str, Option<&str>> = HashMap::new();
    let mut depth: HashMap<&str, usize> = HashMap::new();

    for &start in adj.keys() {
        if visited.contains(start) {
            continue;
        }

        // Iterative DFS with stack: (node, iterator_index)
        let mut stack: Vec<(&str, usize)> = Vec::new();
        stack.push((start, 0));
        visited.insert(start);
        parent.insert(start, None);
        depth.insert(start, 0);

        while let Some(&mut (node, ref mut idx)) = stack.last_mut() {
            let neighbors = adj.get(node).cloned().unwrap_or_default();
            if *idx < neighbors.len() {
                let neighbor = neighbors[*idx];
                *idx += 1;

                if !visited.contains(neighbor) {
                    visited.insert(neighbor);
                    parent.insert(neighbor, Some(node));
                    depth.insert(neighbor, depth[&node] + 1);
                    stack.push((neighbor, 0));
                } else if neighbor != parent[&node].unwrap_or("")
                    && depth[&neighbor] <= depth[&node]
                    && depth[&node] - depth[&neighbor] >= 2
                {
                    // Found a back-edge that indicates a cycle of length >= 3.
                    let mut cycle: Vec<String> = Vec::new();
                    let mut cur = node;
                    loop {
                        cycle.push(cur.to_string());
                        if cur == neighbor {
                            break;
                        }
                        match parent[&cur] {
                            Some(p) => cur = p,
                            None => break,
                        }
                    }
                    if cycle.len() >= 3 {
                        cycle.reverse();
                        // Deduplicate: sort the cycle and check if we've seen it before.
                        let mut sorted_cycle = cycle.clone();
                        sorted_cycle.sort();
                        if !cycles.iter().any(|c| {
                            let mut sc = c.clone();
                            sc.sort();
                            sc == sorted_cycle
                        }) {
                            cycles.push(cycle);
                        }
                    }
                }
            } else {
                stack.pop();
            }
        }
    }

    cycles.truncate(20);
    cycles
}

/// Betweenness centrality approximation via BFS from each node.
pub fn compute_bridge_centrality(edges: &[MemoryGraphEdge]) -> HashMap<String, f64> {
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        adj.entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
        adj.entry(edge.target.as_str())
            .or_default()
            .push(edge.source.as_str());
    }

    let nodes: Vec<&str> = adj.keys().copied().collect();
    let mut centrality: HashMap<String, f64> = HashMap::new();
    for node in &nodes {
        centrality.insert(node.to_string(), 0.0);
    }

    for &s in &nodes {
        // BFS from s
        let mut queue: VecDeque<&str> = VecDeque::new();
        let mut distance: HashMap<&str, i64> = HashMap::new();
        let mut num_shortest: HashMap<&str, f64> = HashMap::new();
        let mut dependency: HashMap<&str, f64> = HashMap::new();

        for &node in &nodes {
            distance.insert(node, -1);
            num_shortest.insert(node, 0.0);
            dependency.insert(node, 0.0);
        }

        distance.insert(s, 0);
        num_shortest.insert(s, 1.0);
        queue.push_back(s);

        let mut stack: Vec<&str> = Vec::new();

        while let Some(v) = queue.pop_front() {
            stack.push(v);
            if let Some(neighbors) = adj.get(v) {
                for &w in neighbors {
                    if distance[w] < 0 {
                        distance.insert(w, distance[v] + 1);
                        queue.push_back(w);
                    }
                    if distance[w] == distance[v] + 1 {
                        *num_shortest.get_mut(w).unwrap() += num_shortest[v];
                    }
                }
            }
        }

        // Accumulate dependencies in reverse order.
        while let Some(v) = stack.pop() {
            if let Some(neighbors) = adj.get(v) {
                for &w in neighbors {
                    if distance[w] == distance[v] + 1 {
                        let contrib = num_shortest[v] / num_shortest[w] * (1.0 + dependency[w]);
                        *dependency.get_mut(v).unwrap() += contrib;
                    }
                }
            }
            if v != s {
                *centrality.get_mut(v).unwrap() += dependency[v];
            }
        }
    }

    centrality
}

/// Build a language breakdown from node names (parse file extensions).
fn build_language_breakdown(nodes: &[MemoryGraphNode]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for node in nodes {
        let ext = node
            .id
            .rsplit('.')
            .next()
            .map(|e| {
                if e.chars().all(|c| c.is_ascii_alphanumeric()) && e.len() <= 8 {
                    e.to_lowercase()
                } else {
                    "other".to_string()
                }
            })
            .unwrap_or_else(|| "other".to_string());
        *counts.entry(ext).or_insert(0) += 1;
    }
    counts
}

/// Find surprising cross-module connections.
fn find_surprising_connections(graph: &MemoryGraph) -> Vec<String> {
    let mut surprising: Vec<String> = Vec::new();
    for edge in &graph.edges {
        let source_ext = edge.source.rsplit('.').next().unwrap_or("").to_lowercase();
        let target_ext = edge.target.rsplit('.').next().unwrap_or("").to_lowercase();
        if !source_ext.is_empty() && !target_ext.is_empty() && source_ext != target_ext {
            surprising.push(format!(
                "{} ({}) <-> {} ({}) [{}]",
                edge.source, source_ext, edge.target, target_ext, edge.kind
            ));
        }
    }
    surprising.truncate(20);
    surprising
}

/// Render the Markdown string.
#[allow(clippy::too_many_arguments)]
fn render_markdown(
    file_count: usize,
    edge_count: usize,
    community_count: usize,
    god_nodes: &[GodNodeEntry],
    bridges: &[BridgeEntry],
    cycles: &[Vec<String>],
    isolation_ratio: f64,
    language_breakdown: &HashMap<String, usize>,
    surprising_connections: &[String],
) -> String {
    let mut md = String::new();
    md.push_str("# Architecture Report\n\n");

    // Overview
    md.push_str("## Overview\n\n");
    md.push_str(&format!("- **Files (nodes):** {}\n", file_count));
    md.push_str(&format!("- **Edges:** {}\n", edge_count));
    md.push_str(&format!("- **Communities:** {}\n", community_count));
    md.push_str(&format!(
        "- **Isolation ratio:** {:.2}%\n\n",
        isolation_ratio * 100.0
    ));

    // Language breakdown
    md.push_str("## Languages\n\n");
    md.push_str("| Extension | Count |\n");
    md.push_str("|-----------|-------|\n");
    let mut langs: Vec<(&String, &usize)> = language_breakdown.iter().collect();
    langs.sort_by(|a, b| b.1.cmp(a.1));
    for (ext, count) in langs {
        md.push_str(&format!("| {} | {} |\n", ext, count));
    }
    md.push('\n');

    // Communities
    md.push_str("## Communities\n\n");
    for (i, community) in god_nodes.chunks(10).enumerate() {
        md.push_str(&format!(
            "Cluster {}: {} members (sampled)\n",
            i + 1,
            community.len()
        ));
    }
    md.push_str(&format!("\nTotal communities: {}\n\n", community_count));

    // God Nodes
    md.push_str("## God Nodes (hub nodes)\n\n");
    md.push_str("| Node | Degree | Kind |\n");
    md.push_str("|------|--------|------|\n");
    for gn in god_nodes {
        md.push_str(&format!(
            "| {} | {} | {} |\n",
            gn.name, gn.edge_count, gn.kind
        ));
    }
    md.push('\n');

    // Bridges
    md.push_str("## Bridges (high-betweenness nodes)\n\n");
    md.push_str("| Node | Betweenness | Kind |\n");
    md.push_str("|------|-------------|------|\n");
    for b in bridges {
        md.push_str(&format!(
            "| {} | {:.4} | {} |\n",
            b.name, b.centrality, b.kind
        ));
    }
    md.push('\n');

    // Cycles
    md.push_str("## Cycles\n\n");
    if cycles.is_empty() {
        md.push_str("No cycles detected.\n\n");
    } else {
        md.push_str(&format!("Found {} cycle(s):\n\n", cycles.len()));
        for (i, cycle) in cycles.iter().enumerate() {
            md.push_str(&format!("{}. `{}`\n", i + 1, cycle.join(" → ")));
        }
        md.push('\n');
    }

    // Surprising Connections
    md.push_str("## Surprising Connections\n\n");
    if surprising_connections.is_empty() {
        md.push_str("No cross-extension connections found.\n");
    } else {
        for conn in surprising_connections {
            md.push_str(&format!("- {}\n", conn));
        }
    }

    md
}

/// Return a map of "YYYY-MM-DD" -> count of memories created on that day.
///
/// `days` controls how far back to look (default 365).
pub fn activity_heatmap(memories: &[Memory], days: usize) -> HashMap<String, u32> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);
    let mut counts: HashMap<String, u32> = HashMap::new();
    for m in memories {
        if m.created_at < cutoff {
            continue;
        }
        let key = m.created_at.format("%Y-%m-%d").to_string();
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}
