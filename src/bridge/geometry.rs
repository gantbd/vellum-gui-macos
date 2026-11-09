//! Read-only UI measurement, never engine geometry authority or hit admission.
use super::*;
use iced::{Size, advanced::text::editor::Selection};

/// Original source-byte targets and the text viewport used by widget layout.
/// Coordinates are not repaired. Results refer only to that completed layout;
/// consumers must not reuse them after source, attachment or layout changes.
pub struct GeometryRequest<'a> {
    pub version: &'a DocumentVersion,
    pub anchor: usize,
    pub head: usize,
    pub text_bounds: Size,
    pub max_regions: usize,
}

/// Content/identity-free UI service populations, not engine commands or timing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GeometryUsage {
    pub offered: u64,
    pub completed: u64,
    pub refused: u64,
    pub emitted_regions: u64,
    pub overflowed: bool,
}

pub(super) struct SecondaryTargets {
    version: DocumentVersion,
    token: ViewToken,
    primary: Cursor,
    cursors: Vec<Cursor>,
}

fn add(value: &mut u64, amount: u64, overflowed: &mut bool) {
    if let Some(next) = value.checked_add(amount) {
        *value = next;
    } else {
        *value = u64::MAX;
        *overflowed = true;
    }
}

impl Bridge {
    pub(super) fn capture_secondary_targets(
        &mut self,
        snapshot: Option<vellum_engine::coordinator::LogicalViewSnapshot>,
    ) -> Result<(), BridgeError> {
        let Some(snapshot) = snapshot else {
            return Ok(());
        };
        let selections = snapshot.selections.selections();
        if selections.len() <= 1 {
            return Ok(());
        }
        if snapshot.source.as_ref() != Some(&self.original.version) {
            return Err(BridgeError::Stale);
        }
        let primary = selections[snapshot.selections.primary()];
        let head = self
            .projection
            .position(&self.original.text, primary.head.0)?;
        let anchor = self
            .projection
            .position(&self.original.text, primary.anchor.0)?;
        if self.content.cursor()
            != (Cursor {
                position: head,
                selection: (head != anchor).then_some(anchor),
            })
        {
            return Err(BridgeError::ProjectionMismatch);
        }
        let mut cursors = Vec::new();
        cursors
            .try_reserve_exact(selections.len() - 1)
            .map_err(|_| BridgeError::Capacity)?;
        for (index, selection) in selections.iter().enumerate() {
            if index == snapshot.selections.primary() {
                continue;
            }
            let head = self
                .projection
                .position(&self.original.text, selection.head.0)?;
            let anchor = self
                .projection
                .position(&self.original.text, selection.anchor.0)?;
            cursors.push(Cursor {
                position: head,
                selection: (head != anchor).then_some(anchor),
            });
        }
        self.secondary_targets = Some(SecondaryTargets {
            version: self.original.version.clone(),
            token: self.token(),
            primary: self.content.cursor(),
            cursors,
        });
        Ok(())
    }

    /// UI draw-time read: exact native targets captured with the source projection.
    /// No engine query, shaping or cursor mutation occurs during painting. The
    /// shared region ceiling covers the whole result, not each target separately.
    pub fn secondary_geometry(
        &self,
        text_bounds: Size,
        max_regions: usize,
    ) -> Result<Vec<Selection>, BridgeError> {
        let result = (|| {
            if self.is_closed() {
                return Err(BridgeError::Stale);
            }
            let Some(targets) = &self.secondary_targets else {
                if self.has_secondary_selections {
                    return Err(BridgeError::GeometryUnavailable);
                }
                return Ok(Vec::new());
            };
            if targets.version != self.original.version
                || targets.token != self.token
                || targets.primary != self.content.cursor()
            {
                return Err(BridgeError::Stale);
            }
            if max_regions < targets.cursors.len() {
                return Err(BridgeError::Capacity);
            }
            let mut output = Vec::new();
            output
                .try_reserve_exact(targets.cursors.len())
                .map_err(|_| BridgeError::Capacity)?;
            let mut remaining = max_regions;
            for cursor in &targets.cursors {
                let selection = self
                    .content
                    .selection_geometry(text_bounds, *cursor, remaining)
                    .ok_or(BridgeError::GeometryUnavailable)?;
                remaining -= match &selection {
                    Selection::Caret(_) => 1,
                    Selection::Range(r) => r.len(),
                };
                output.push(selection);
            }
            Ok(output)
        })();
        let mut usage = self.secondary_usage.get();
        add(&mut usage.offered, 1, &mut usage.overflowed);
        match &result {
            Ok(selections) => {
                add(&mut usage.completed, 1, &mut usage.overflowed);
                let regions = selections
                    .iter()
                    .map(|s| match s {
                        Selection::Caret(_) => 1,
                        Selection::Range(r) => r.len(),
                    })
                    .sum::<usize>();
                add(
                    &mut usage.emitted_regions,
                    regions as u64,
                    &mut usage.overflowed,
                );
            }
            Err(_) => add(&mut usage.refused, 1, &mut usage.overflowed),
        }
        self.secondary_usage.set(usage);
        result
    }

    pub fn secondary_geometry_usage(&self) -> GeometryUsage {
        self.secondary_usage.get()
    }

    /// Query the actual renderer without moving its primary cursor or issuing
    /// an engine command. The caller adds padding/translation and clips results
    /// to its current viewport. Missing layout/capacity refuses the whole read.
    pub fn rendered_geometry(
        &mut self,
        token: &ViewToken,
        request: GeometryRequest<'_>,
    ) -> Result<Selection, BridgeError> {
        let result = self.read_rendered_geometry(token, request);
        let usage = &mut self.geometry_usage;
        add(&mut usage.offered, 1, &mut usage.overflowed);
        match &result {
            Ok(selection) => {
                add(&mut usage.completed, 1, &mut usage.overflowed);
                let regions = match selection {
                    Selection::Caret(_) => 1,
                    Selection::Range(regions) => regions.len(),
                };
                add(
                    &mut usage.emitted_regions,
                    regions as u64,
                    &mut usage.overflowed,
                );
            }
            Err(_) => add(&mut usage.refused, 1, &mut usage.overflowed),
        }
        result
    }

    pub fn geometry_usage(&self) -> GeometryUsage {
        self.geometry_usage
    }

    fn read_rendered_geometry(
        &mut self,
        token: &ViewToken,
        request: GeometryRequest<'_>,
    ) -> Result<Selection, BridgeError> {
        self.check(token)?;
        if request.version != &self.original.version {
            return Err(BridgeError::Stale);
        }
        if request.max_regions == 0 {
            return Err(BridgeError::Capacity);
        }
        let head = self
            .projection
            .position(&self.original.text, request.head)?;
        let anchor = self
            .projection
            .position(&self.original.text, request.anchor)?;
        self.content
            .selection_geometry(
                request.text_bounds,
                Cursor {
                    position: head,
                    selection: (anchor != head).then_some(anchor),
                },
                request.max_regions,
            )
            .ok_or(BridgeError::GeometryUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_counters_saturate_with_a_sticky_overflow_indicator() {
        let mut value = u64::MAX;
        let mut overflowed = false;
        add(&mut value, 1, &mut overflowed);
        assert_eq!(value, u64::MAX);
        assert!(overflowed);
        add(&mut value, 0, &mut overflowed);
        assert!(overflowed);
    }
}
