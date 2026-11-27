//! Milestone `M2` 4-Layer Provenance-Locked Causal Knowledge Graph (`SUB-04` Loom / `fnx-*` design).
//!
//! Layers:
//! - `L0Verified`: verified factual/vintage entities & primary events (strict DAG)
//! - `L1Mechanism`: structural causal pathways (strict DAG)
//! - `L2Signpost`: observable leading indicators & tripwires
//! - `L3Hypothesis`: competing epistemic hypotheses
//!
//! Enforces `INV-03` (`available_utc <= as_of_utc`) on subgraph projection, DAG acyclicity on
//! `L0`/`L1`, and deterministic 1-hop causal spillover wakeups across linked questions.

use fcast_core::blinding::VerifiedEvidenceRef;
use fcast_core::time::{is_utc_le, parse_utc_epoch_ms};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum GraphError {
    #[error("unknown causal node: {0}")]
    UnknownNode(String),
    #[error("duplicate causal node_id: {0}")]
    DuplicateNode(String),
    #[error("duplicate causal edge: {from} -> {to} on {layer:?}")]
    DuplicateEdge {
        layer: GraphLayer,
        from: String,
        to: String,
    },
    #[error("invalid causal edge polarity {0}: expected -1 or +1")]
    InvalidPolarity(i8),
    #[error("invalid causal edge strength {0}: expected finite value in (0.0, 1.0]")]
    InvalidStrength(f64),
    #[error("invalid UTC timestamp in causal graph: {0}")]
    InvalidTimestamp(String),
    #[error("DAG cycle detected on layer {layer:?}: edge {from} -> {to}")]
    CycleDetected {
        layer: GraphLayer,
        from: String,
        to: String,
    },
}

/// Four epistemic layers of the `Loom` causal Knowledge Graph (`§11`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphLayer {
    L0Verified,
    L1Mechanism,
    L2Signpost,
    L3Hypothesis,
}

impl GraphLayer {
    #[must_use]
    pub const fn requires_dag(self) -> bool {
        matches!(self, Self::L0Verified | Self::L1Mechanism)
    }
}

/// One provenance-locked node in the 4-layer causal graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CausalNode {
    pub node_id: String,
    pub layer: GraphLayer,
    pub label: String,
    /// Optional linked forecasting question ID for 1-hop spillover wakeups.
    pub question_id: Option<String>,
    pub ev_handle: VerifiedEvidenceRef,
    pub available_utc: String,
}

/// Directed causal edge with signed polarity (`+1` amplifying, `-1` dampening) and strength `(0, 1]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalEdge {
    pub from_id: String,
    pub to_id: String,
    pub layer: GraphLayer,
    pub polarity: i8,
    pub strength: f64,
    pub ev_handle: VerifiedEvidenceRef,
    pub available_utc: String,
}

/// Deterministic 1-hop causal spillover wakeup emitted when an upstream question shifts belief.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpilloverWakeup {
    pub source_question_id: String,
    pub target_question_id: String,
    pub via_edge_from: String,
    pub via_edge_to: String,
    pub propagated_delta_logit: f64,
    pub ev_handle: VerifiedEvidenceRef,
}

/// Pure-Rust 4-layer causal Knowledge Graph with exact bi-temporal projection and spillover queries.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CausalGraph {
    nodes: BTreeMap<String, CausalNode>,
    edges: Vec<CausalEdge>,
}

impl CausalGraph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, node: CausalNode) -> Result<(), GraphError> {
        parse_utc_epoch_ms(&node.available_utc)
            .map_err(|_| GraphError::InvalidTimestamp(node.available_utc.clone()))?;
        if self.nodes.contains_key(&node.node_id) {
            return Err(GraphError::DuplicateNode(node.node_id));
        }
        self.nodes.insert(node.node_id.clone(), node);
        Ok(())
    }

    pub fn add_edge(&mut self, edge: CausalEdge) -> Result<(), GraphError> {
        if !self.nodes.contains_key(&edge.from_id) {
            return Err(GraphError::UnknownNode(edge.from_id));
        }
        if !self.nodes.contains_key(&edge.to_id) {
            return Err(GraphError::UnknownNode(edge.to_id));
        }
        if edge.polarity != -1 && edge.polarity != 1 {
            return Err(GraphError::InvalidPolarity(edge.polarity));
        }
        if !edge.strength.is_finite() || edge.strength <= 0.0 || edge.strength > 1.0 {
            return Err(GraphError::InvalidStrength(edge.strength));
        }
        parse_utc_epoch_ms(&edge.available_utc)
            .map_err(|_| GraphError::InvalidTimestamp(edge.available_utc.clone()))?;

        if self
            .edges
            .iter()
            .any(|e| e.from_id == edge.from_id && e.to_id == edge.to_id && e.layer == edge.layer)
        {
            return Err(GraphError::DuplicateEdge {
                layer: edge.layer,
                from: edge.from_id,
                to: edge.to_id,
            });
        }
        if edge.layer.requires_dag() && self.would_create_dag_cycle(&edge.from_id, &edge.to_id) {
            return Err(GraphError::CycleDetected {
                layer: edge.layer,
                from: edge.from_id,
                to: edge.to_id,
            });
        }
        self.edges.push(edge);
        Ok(())
    }

    /// Re-validates all nodes, edges, timestamps, and `L0`/`L1` DAG acyclicity (e.g., after JSON deserialization).
    pub fn validate(&self) -> Result<(), GraphError> {
        let mut rebuilt = Self::new();
        for node in self.nodes.values() {
            rebuilt.add_node(node.clone())?;
        }
        for edge in &self.edges {
            rebuilt.add_edge(edge.clone())?;
        }
        Ok(())
    }

    /// Returns a strictly time-fenced subgraph retaining ONLY nodes and edges with
    /// `available_utc <= as_of_utc` (`INV-03`).
    pub fn project_as_of(&self, as_of_utc: &str) -> Result<Self, GraphError> {
        parse_utc_epoch_ms(as_of_utc)
            .map_err(|_| GraphError::InvalidTimestamp(as_of_utc.to_owned()))?;
        let mut nodes = BTreeMap::new();
        for (id, node) in &self.nodes {
            if is_utc_le(&node.available_utc, as_of_utc) {
                nodes.insert(id.clone(), node.clone());
            }
        }
        let edges = self
            .edges
            .iter()
            .filter(|e| {
                is_utc_le(&e.available_utc, as_of_utc)
                    && nodes.contains_key(&e.from_id)
                    && nodes.contains_key(&e.to_id)
            })
            .cloned()
            .collect();
        Ok(Self { nodes, edges })
    }

    /// Computes deterministic 1-hop causal spillover wakeups from `source_question_id`
    /// whenever `|delta_logit * polarity * strength| >= wake_threshold_abs` (`wake_threshold_abs > 0`).
    #[must_use]
    pub fn spillover_targets(
        &self,
        source_question_id: &str,
        delta_logit: f64,
        wake_threshold_abs: f64,
    ) -> Vec<SpilloverWakeup> {
        if !delta_logit.is_finite()
            || !wake_threshold_abs.is_finite()
            || wake_threshold_abs < 0.0
            || delta_logit == 0.0
        {
            return Vec::new();
        }
        let source_nodes: BTreeSet<&str> = self
            .nodes
            .values()
            .filter(|n| n.question_id.as_deref() == Some(source_question_id))
            .map(|n| n.node_id.as_str())
            .collect();

        let mut wakeups = Vec::new();
        for edge in &self.edges {
            if !source_nodes.contains(edge.from_id.as_str()) {
                continue;
            }
            let Some(target_node) = self.nodes.get(&edge.to_id) else {
                continue;
            };
            let Some(target_q) = &target_node.question_id else {
                continue;
            };
            if target_q == source_question_id {
                continue;
            }
            let propagated = delta_logit * f64::from(edge.polarity) * edge.strength;
            if propagated.abs() >= wake_threshold_abs {
                wakeups.push(SpilloverWakeup {
                    source_question_id: source_question_id.to_owned(),
                    target_question_id: target_q.clone(),
                    via_edge_from: edge.from_id.clone(),
                    via_edge_to: edge.to_id.clone(),
                    propagated_delta_logit: propagated,
                    ev_handle: edge.ev_handle.clone(),
                });
            }
        }
        wakeups.sort_by(|a, b| {
            b.propagated_delta_logit
                .abs()
                .total_cmp(&a.propagated_delta_logit.abs())
                .then_with(|| a.target_question_id.cmp(&b.target_question_id))
                .then_with(|| a.via_edge_from.cmp(&b.via_edge_from))
                .then_with(|| a.via_edge_to.cmp(&b.via_edge_to))
                .then_with(|| a.ev_handle.cmp(&b.ev_handle))
        });
        wakeups
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    fn would_create_dag_cycle(&self, from_id: &str, to_id: &str) -> bool {
        if from_id == to_id {
            return true;
        }
        // Build O(E) adjacency list once so reachability DFS runs in O(V + E).
        let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for e in &self.edges {
            if e.layer.requires_dag() {
                adj.entry(e.from_id.as_str())
                    .or_default()
                    .push(e.to_id.as_str());
            }
        }
        let mut stack = vec![to_id];
        let mut visited = BTreeSet::new();
        while let Some(curr) = stack.pop() {
            if curr == from_id {
                return true;
            }
            if !visited.insert(curr) {
                continue;
            }
            if let Some(neighbors) = adj.get(curr) {
                stack.extend(neighbors.iter().copied());
            }
        }
        false
    }
}
