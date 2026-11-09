//! Stable, reusable instance groups for whole-model draws.

#[derive(Default)]
pub(super) struct InstanceGroups {
    offsets: Vec<usize>,
    pub order: Vec<u32>,
}

impl InstanceGroups {
    /// Groups input indices by model, preserving their input order within each model.
    /// Invalid models and single-piece instances (`None`) do not enter these groups.
    pub fn build(&mut self, models: usize, keys: impl Iterator<Item = Option<usize>> + Clone) {
        self.offsets.clear();
        self.offsets.resize(models + 1, 0);
        for model in keys.clone().flatten().filter(|&m| m < models) {
            self.offsets[model + 1] += 1;
        }
        // The extra leading zero lets the scatter cursors become the final group ends.
        let mut total = 0;
        for offset in &mut self.offsets {
            let count = *offset;
            *offset = total;
            total += count;
        }
        self.order.resize(total, 0);
        for (index, model) in keys.enumerate() {
            let Some(model) = model.filter(|&m| m < models) else { continue };
            let next = &mut self.offsets[model + 1];
            self.order[*next] = index as u32;
            *next += 1;
        }
    }

    pub fn groups(&self) -> impl Iterator<Item = &[u32]> {
        self.offsets.windows(2).map(|r| &self.order[r[0]..r[1]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn previous(models: usize, keys: &[Option<usize>]) -> Vec<Vec<u32>> {
        let mut groups = vec![Vec::new(); models];
        for (index, &model) in keys.iter().enumerate() {
            if let Some(group) = model.and_then(|m| groups.get_mut(m)) {
                group.push(index as u32);
            }
        }
        groups
    }

    #[test]
    fn reusable_groups_preserve_model_and_instance_order() {
        let mut groups = InstanceGroups::default();
        for (models, count) in [(701, 20_000), (7, 100), (0, 20), (13, 0), (701, 20_000)] {
            let keys: Vec<_> = (0..count).map(|i| (i % 9 != 0).then_some((i * 7919) % (models + 3))).collect();
            groups.build(models, keys.iter().copied());
            let expected = previous(models, &keys);
            assert_eq!(groups.groups().collect::<Vec<_>>(), expected.iter().map(Vec::as_slice).collect::<Vec<_>>());
            assert_eq!(groups.order, expected.into_iter().flatten().collect::<Vec<_>>());
        }
    }

    #[test]
    fn repeated_frames_reuse_the_group_storage() {
        let keys = [Some(2), None, Some(0), Some(2), Some(6), Some(99)];
        let mut groups = InstanceGroups::default();
        groups.build(8, keys.into_iter());
        let pointers = (groups.order.as_ptr(), groups.offsets.as_ptr());
        groups.build(8, keys.into_iter());
        assert_eq!(pointers, (groups.order.as_ptr(), groups.offsets.as_ptr()));
        assert_eq!(groups.order, [2, 0, 3, 4]);
    }
}
