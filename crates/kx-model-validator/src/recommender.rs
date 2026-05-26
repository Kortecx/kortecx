//! [`Recommender`] — thin layer over [`crate::check`] + a [`ModelRegistry`]
//! that returns ranked candidates per a [`RankingPolicy`]. Suggests, never
//! substitutes — the principle mirrors the broker (P1.8.5) never silently
//! rewriting a tool call.

use kx_mote::ModelId;

use crate::check::check;
use crate::outcome::ValidatorOutcome;
use crate::provided::ProvidedCapabilities;
use crate::registry::ModelRegistry;
use crate::requirements::RequiredCapabilities;

/// Ranking authority used by [`Recommender::candidates`].
///
/// Strict precedence (D29):
/// 1. **Deterministic capability match** (always). Models that fail the
///    check are filtered out; among the remaining, TypeOk outranks
///    DegradedSubtype, and within each tier, models with more soft
///    preferences satisfied outrank fewer.
/// 2. **Workflow-named eval score** (optional). If the caller names an eval,
///    higher scores rank higher within tier 1.
/// 3. **In-runtime Mote-measured performance** (deferred to P1.13+). v1
///    returns a stable order within the tier; v2 will plug a journal-derived
///    score in here.
///
/// **The recommender does NOT know which model is the house model.** No
/// flag, no boost, no field. Type framing makes no-favoritism structural.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RankingPolicy {
    /// If `Some(name)`, the recommender ranks acceptable candidates by
    /// `provided.eval_scores[name]` descending. Caller-chosen, never picked
    /// by the recommender.
    pub eval_metric: Option<String>,
}

/// One candidate result from the recommender.
///
/// Sorted within a `Vec<Candidate>` by the [`RankingPolicy`] precedence.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// The candidate's identity.
    pub model_id: ModelId,
    /// The check outcome against the workflow's `required` capabilities.
    pub outcome: ValidatorOutcome,
    /// The candidate's declared capabilities (carried for the caller's
    /// inspection — e.g., showing why one ranked above another).
    pub provided: ProvidedCapabilities,
    /// The eval score under the workflow's `RankingPolicy.eval_metric`, if
    /// present and applicable.
    pub eval_score: Option<f32>,
}

/// Thin layer over [`crate::check`] + a [`ModelRegistry`]. Suggests, never
/// substitutes.
///
/// Same principle as the broker (P1.8.5) never silently rewriting a tool
/// call: the recommender returns ranked candidates; the caller (SDK or
/// runtime router) decides whether to act on the suggestion.
///
/// # Examples
///
/// ```
/// use kx_model_validator::{
///     check, InMemoryModelRegistry, ProvidedCapabilities, Quantization,
///     RankingPolicy, Recommender, RequiredCapabilities, ValidatorOutcome,
/// };
/// use kx_mote::ModelId;
/// use std::collections::BTreeSet;
///
/// let mut reg = InMemoryModelRegistry::new();
/// reg.insert(
///     ModelId("small-text".into()),
///     ProvidedCapabilities::declared().with_context_window_tokens(2_048),
/// );
/// reg.insert(
///     ModelId("big-text".into()),
///     ProvidedCapabilities::declared().with_context_window_tokens(32_768),
/// );
///
/// let req = RequiredCapabilities {
///     min_context_window_tokens: 16_000,
///     ..RequiredCapabilities::permissive()
/// };
///
/// let recommender = Recommender::new(&reg, RankingPolicy::default());
/// let candidates = recommender.candidates(&req);
///
/// // Only big-text satisfies the 16k context requirement.
/// assert_eq!(candidates.len(), 1);
/// assert_eq!(candidates[0].model_id, ModelId("big-text".into()));
/// assert_eq!(candidates[0].outcome, ValidatorOutcome::TypeOk);
/// ```
pub struct Recommender<'a, R: ModelRegistry + ?Sized> {
    registry: &'a R,
    policy: RankingPolicy,
}

impl<'a, R: ModelRegistry + ?Sized> Recommender<'a, R> {
    /// Construct a recommender over `registry` with the given ranking
    /// `policy`.
    pub fn new(registry: &'a R, policy: RankingPolicy) -> Self {
        Self { registry, policy }
    }

    /// Check one specific model against the requirements.
    pub fn check_model(
        &self,
        model_id: &ModelId,
        required: &RequiredCapabilities,
    ) -> Option<Candidate> {
        let provided = self.registry.lookup(model_id)?;
        let outcome = check(&provided, required);
        let eval_score = self.eval_score_of(&provided);
        Some(Candidate {
            model_id: model_id.clone(),
            outcome,
            provided,
            eval_score,
        })
    }

    /// Return every model in the registry whose check is `TypeOk` or
    /// `DegradedSubtype`, ranked by the configured `RankingPolicy`.
    ///
    /// TypeError candidates are NOT returned — the caller asked for
    /// "candidates that could bind."
    pub fn candidates(&self, required: &RequiredCapabilities) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = self
            .registry
            .entries()
            .into_iter()
            .filter_map(|(id, provided)| {
                let outcome = check(&provided, required);
                if outcome.is_acceptable() {
                    let eval_score = self.eval_score_of(&provided);
                    Some(Candidate {
                        model_id: id,
                        outcome,
                        provided,
                        eval_score,
                    })
                } else {
                    None
                }
            })
            .collect();

        // Stable sort: TypeOk before DegradedSubtype; within each, higher
        // eval score first (if eval_metric is set); otherwise ModelId
        // ascending (deterministic, no implicit preference).
        out.sort_by(|a, b| {
            let a_rank = outcome_rank(&a.outcome);
            let b_rank = outcome_rank(&b.outcome);
            a_rank.cmp(&b_rank).then_with(|| {
                // Higher eval score first → compare b vs a.
                match (a.eval_score, b.eval_score) {
                    (Some(av), Some(bv)) => {
                        bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.model_id.cmp(&b.model_id),
                }
            })
        });

        out
    }

    fn eval_score_of(&self, provided: &ProvidedCapabilities) -> Option<f32> {
        self.policy
            .eval_metric
            .as_deref()
            .and_then(|name| provided.eval_scores.get(name).copied())
    }
}

/// Numeric rank for outcome sorting: lower = better.
fn outcome_rank(o: &ValidatorOutcome) -> u8 {
    match o {
        ValidatorOutcome::TypeOk => 0,
        ValidatorOutcome::DegradedSubtype { .. } => 1,
        ValidatorOutcome::TypeError { .. } => 2,
    }
}
