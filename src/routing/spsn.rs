use crate::{
    bundle::Bundle,
    contact_manager::ContactManager,
    contact_plan::ContactPlan,
    errors::ASABRError,
    multigraph::Multigraph,
    node_manager::NodeManager,
    pathfinding::Pathfinding,
    route_storage::{Guard, TreeStorage},
    types::{Date, NodeID},
};

use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use super::{Router, RoutingOutput, schedule_multicast, schedule_unicast};

/// A structure representing the Shortest Path with Safety Nodes (SPSN) algorithm.
///
/// This struct handles routing logic and pathfinding, utilizing stored routes
/// and ensuring that the routing process adheres to specified safety and priority constraints.
///
/// # Type Parameters
/// - `NM`: A type that implements the `NodeManager` trait, responsible for managing the
///   network's nodes and their interactions.
/// - `CM`: A type that implements the `ContactManager` trait, handling contact points and
///   communication schedules within the network.
/// - `P`: A type that implements the `Pathfinding<NM, CM>` trait, responsible for computing optimal paths.
pub struct Spsn<NM: NodeManager, CM: ContactManager, P: Pathfinding<NM, CM>, S: TreeStorage<NM, CM>>
{
    /// A reference-counted storage for routing data, allowing the retrieval and storage of
    /// pathfinding output.
    route_storage: Rc<RefCell<S>>,
    /// The pathfinding instance used for route calculations, responsible for computing optimal
    /// paths based on the current network state.
    pathfinding: P,
    /// The guard structure that enforces safety and priority constraints, checking if the routing
    /// can proceed based on the current bundle and its constraints.
    unicast_guard: Guard,

    // for compilation
    #[doc(hidden)]
    _phantom_nm: PhantomData<NM>,
    #[doc(hidden)]
    _phantom_cm: PhantomData<CM>,
}

impl<NM: NodeManager, CM: ContactManager, P: Pathfinding<NM, CM>, S: TreeStorage<NM, CM>>
    Router<NM, CM> for Spsn<NM, CM, P, S>
{
    fn route(
        &mut self,
        source: NodeID,
        bundle: &Bundle,
        curr_time: Date,
        excluded_nodes: &[NodeID],
    ) -> Result<Option<RoutingOutput<NM, CM>>, ASABRError> {
        if bundle.expiration < curr_time {
            return Ok(None);
        }

        if bundle.destinations.len() == 1 {
            return self.route_unicast(source, bundle, curr_time, excluded_nodes);
        }

        self.route_multicast(source, bundle, curr_time, excluded_nodes)
    }
}

impl<S: TreeStorage<NM, CM>, NM: NodeManager, CM: ContactManager, P: Pathfinding<NM, CM>>
    Spsn<NM, CM, P, S>
{
    /// Creates a new `Spsn` instance with the specified parameters.
    ///
    /// # Parameters
    ///
    /// * `ContactPlan` - A contact plan of nodes representing the routing network, contacts and a
    ///   vnode map, and associated management information.
    /// * `route_storage` - A reference-counted storage for routing data.
    /// * `with_priorities` - A boolean indicating whether to consider priorities during routing.
    ///
    /// # Returns
    ///
    /// * `Self` - A new instance of the `Spsn` struct.
    pub fn new(
        contact_plan: ContactPlan<NM, CM>,
        route_storage: Rc<RefCell<S>>,
        with_priorities: bool,
    ) -> Result<Self, ASABRError> {
        Ok(Self {
            pathfinding: P::new(Rc::new(RefCell::new(Multigraph::new(contact_plan)?))),
            route_storage: route_storage.clone(),
            unicast_guard: Guard::new(with_priorities),
            // for compilation
            _phantom_nm: PhantomData,
            _phantom_cm: PhantomData,
        })
    }

    /// Routes a bundle to a single destination node using unicast routing.
    ///
    /// The `route_unicast` function performs a unicast routing operation for bundles with only
    /// one destination. It first checks if the unicast operation should be aborted (via `unicast_guard`).
    /// Then, it attempts to retrieve or compute a unicast tree. Finally, it schedules unicast routing
    /// using `schedule_unicast`.
    ///
    /// # Parameters
    /// - `source`: The source node ID initiating the unicast routing.
    /// - `bundle`: The `Bundle` containing the single destination and related routing data.
    /// - `curr_time`: The current time for scheduling calculations.
    /// - `excluded_nodes`: A list of nodes to exclude from the unicast path.
    ///
    /// # Returns
    /// An `Result<Option<RoutingOutput<NM, CM>>, ASABRError>` containing the routing result, or `None` if routing fails or
    /// is aborted, or an error if the operation fails.
    fn route_unicast(
        &mut self,
        source: NodeID,
        bundle: &Bundle,
        curr_time: Date,
        excluded_nodes: &[NodeID],
    ) -> Result<Option<RoutingOutput<NM, CM>>, ASABRError> {
        if self.unicast_guard.must_abort(bundle) {
            return Ok(None);
        }

        let dest = bundle.destinations[0];

        let (tree_option, _reachable_nodes) =
            self.route_storage
                .borrow()
                .select(bundle, curr_time, excluded_nodes)?;

        if let Some(tree) = tree_option {
            return Ok(Some(schedule_unicast(bundle, curr_time, tree, false)?));
        }

        let new_tree = self
            .pathfinding
            .get_next(curr_time, source, bundle, excluded_nodes)?;
        let tree_ref = Rc::new(RefCell::new(new_tree));

        self.route_storage
            .try_borrow_mut()?
            .store(bundle, tree_ref.clone());

        match &tree_ref.borrow().by_destination[dest as usize] {
            // The tree is fresh, no dry run was performed, the remained expected fail case is bundle expiration
            // Trees are not built while considering expirations for flexibility
            // /!\ But maybe it should, issues expected with non-SABR distances
            Some(route) => {
                if route.borrow().at_time > bundle.expiration {
                    return Ok(None);
                }
            }
            None => {
                self.unicast_guard.add_limit(bundle, dest as NodeID);
                return Ok(None);
            }
        }

        Ok(Some(schedule_unicast(bundle, curr_time, tree_ref, true)?))
    }

    /// Routes a bundle to multiple destination nodes using multicast routing.
    ///
    /// The `route_multicast` function performs multicast routing when `bundle` has multiple
    /// destinations. It first checks for a pre-existing multicast tree. If a tree exists and
    /// reaches all destinations, it schedules multicast routing using `schedule_multicast`.
    /// Otherwise, it creates a new multicast tree and proceeds to schedule the multicast operation.
    ///
    /// # Parameters
    /// - `source`: The source node ID initiating the multicast routing.
    /// - `bundle`: The `Bundle` containing multiple destinations.
    /// - `curr_time`: The current time for scheduling calculations.
    /// - `excluded_nodes`: A list of nodes to exclude from the multicast paths.
    ///
    /// # Returns
    /// An `Result<Option<RoutingOutput<NM, CM>>, ASABRError>` containing the multicast routing result, or `None` if
    /// routing fails, or an error if the operation fails.
    pub fn route_multicast(
        &mut self,
        source: NodeID,
        bundle: &Bundle,
        curr_time: Date,
        excluded_nodes: &[NodeID],
    ) -> Result<Option<RoutingOutput<NM, CM>>, ASABRError> {
        if let (Some(tree), Some(reachable_nodes)) =
            self.route_storage
                .borrow()
                .select(bundle, curr_time, excluded_nodes)?
            && bundle.destinations.len() == reachable_nodes.len()
        {
            return Ok(Some(schedule_multicast(
                bundle,
                curr_time,
                tree,
                Some(reachable_nodes),
            )?));
        }

        let new_tree = self
            .pathfinding
            .get_next(curr_time, source, bundle, excluded_nodes)?;
        let tree = Rc::new(RefCell::new(new_tree));
        self.route_storage
            .try_borrow_mut()?
            .store(bundle, tree.clone());

        Ok(Some(schedule_multicast(bundle, curr_time, tree, None)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_manager::legacy::evl::EVLManager;
    use crate::distance::hop::Hop;
    use crate::errors::ASABRError;
    use crate::node_manager::none::NoManagement;
    use crate::pathfinding::node_parenting::NodeParentingTreeExcl;
    use crate::pathfinding::test_helpers::*;
    use crate::route_storage::cache::TreeCache;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn make_cache(max: usize) -> Rc<RefCell<TreeCache<NoManagement, EVLManager>>> {
        Rc::new(RefCell::new(TreeCache::new(false, false, max)))
    }

    fn make_spsn(
        with_priorities: bool,
    ) -> Spsn<
        NoManagement,
        EVLManager,
        NodeParentingTreeExcl<NoManagement, EVLManager, Hop>,
        TreeCache<NoManagement, EVLManager>,
    > {
        Spsn::<
            NoManagement,
            EVLManager,
            NodeParentingTreeExcl<NoManagement, EVLManager, Hop>,
            TreeCache<NoManagement, EVLManager>,
        >::new(make_cp(), make_cache(10), with_priorities)
        .unwrap()
    }

    #[test]
    fn test_new_valid() -> Result<(), ASABRError> {
        let cache = make_cache(10);
        let result = Spsn::<
            NoManagement,
            EVLManager,
            NodeParentingTreeExcl<NoManagement, EVLManager, Hop>,
            TreeCache<NoManagement, EVLManager>,
        >::new(make_cp(), cache, false);

        assert!(
            result.is_ok(),
            "TEST FAILED: Spsn::new() should succeed with a valid contact plan."
        );
        Ok(())
    }

    #[test]
    fn test_route_expired_bundle() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(false);
        let bundle = make_bundle(2, 1, 1.0, 10.0);
        let result = spsn.route(0, &bundle, 20.0, &[])?;

        assert!(
            result.is_none(),
            "TEST FAILED: Expired bundle should return None."
        );
        Ok(())
    }

    #[test]
    fn test_route_unicast_unreachable_dest() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(false);
        let bundle = make_bundle(4, 1, 5.0, 1000.0);
        let result = spsn.route(0, &bundle, 0.0, &[1, 2, 3])?;

        assert!(
            result.is_none(),
            "TEST FAILED: Unreachable destination should return None."
        );
        Ok(())
    }

    #[test]
    fn test_route_unicast_guard_aborts() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(false);
        let bundle_large = make_bundle(4, 1, 5.0, 1000.0);
        spsn.route(0, &bundle_large, 0.0, &[1, 2, 3])?;
        let bundle_small = make_bundle(4, 1, 1.0, 1000.0);
        let result = spsn.route(0, &bundle_small, 0.0, &[])?;

        assert!(
            result.is_none(),
            "TEST FAILED: Guard should abort routing for a bundle smaller than the known limit."
        );
        Ok(())
    }

    #[test]
    fn test_route_unicast_cache_hit() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(false);
        let bundle = make_bundle(2, 1, 1.0, 1000.0);
        let result1 = spsn.route(0, &bundle, 0.0, &[])?;

        assert!(
            result1.is_some(),
            "TEST FAILED: First routing should succeed."
        );

        let result2 = spsn.route(0, &bundle, 0.0, &[])?;

        assert!(
            result2.is_some(),
            "TEST FAILED: Second routing should succeed via cache."
        );
        assert!(
            result1.unwrap().lazy_get_for_unicast(2).is_some(),
            "TEST FAILED: First route should reach node 2."
        );
        assert!(
            result2.unwrap().lazy_get_for_unicast(2).is_some(),
            "TEST FAILED: Second route should reach node 2."
        );
        Ok(())
    }

    #[test]
    fn test_route_unicast_path_found_but_expired() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(false);
        let bundle = make_bundle(2, 1, 1.0, 5.0);
        let result = spsn.route(0, &bundle, 0.0, &[])?;

        assert!(
            result.is_none(),
            "TEST FAILED: Bundle expiring before arrival should return None."
        );

        let bundle_valid = make_bundle(2, 1, 1.0, 1000.0);
        let result_valid = spsn.route(0, &bundle_valid, 0.0, &[])?;

        assert!(
            result_valid.is_some(),
            "TEST FAILED: Guard should not have been updated when the bundle expires before arrival."
        );
        Ok(())
    }

    #[test]
    fn test_route_unicast_fresh_tree() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(false);
        let bundle = make_bundle(2, 1, 1.0, 1000.0);
        let result = spsn.route(0, &bundle, 0.0, &[])?;

        assert!(
            result.is_some(),
            "TEST FAILED: Fresh tree routing should return Some for a reachable destination."
        );

        let (_contact, dest_stage) = result
            .unwrap()
            .lazy_get_for_unicast(2)
            .expect("TEST FAILED: RoutingOutput should contain a route to node 2.");

        assert_eq!(
            dest_stage.borrow().to_node,
            2,
            "TEST FAILED: Destination stage should point to node 2."
        );
        assert_eq!(
            dest_stage.borrow().hop_count,
            1,
            "TEST FAILED: Hop distance selects direct path 0 -> 2, should only be 1 hop."
        );
        assert_eq!(
            dest_stage.borrow().at_time,
            26.0,
            "TEST FAILED: Direct path 0 -> 2, arrival should be 26.0."
        );
        Ok(())
    }

    #[test]
    fn test_route_multicast_cache_miss() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(false);
        let bundle = make_multicast_bundle(vec![2, 4], 1, 1.0, 1000.0);
        let result = spsn.route(0, &bundle, 0.0, &[])?;

        assert!(
            result.is_some(),
            "TEST FAILED: Multicast should succeed for reachable destinations."
        );
        assert!(
            !result.unwrap().first_hops.is_empty(),
            "TEST FAILED: RoutingOutput should contain at least one first hop."
        );
        Ok(())
    }

    #[test]
    fn test_route_multicast_cache_hit_full() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(false);
        let bundle = make_multicast_bundle(vec![2, 4], 1, 1.0, 1000.0);
        let result1 = spsn.route(0, &bundle, 0.0, &[])?;

        assert!(
            result1.is_some(),
            "TEST FAILED: First multicast should succeed."
        );
        let result2 = spsn.route(0, &bundle, 0.0, &[])?;
        assert!(
            result2.is_some(),
            "TEST FAILED: Second multicast should succeed via full cache hit."
        );
        assert!(
            !result2.unwrap().first_hops.is_empty(),
            "TEST FAILED: RoutingOutput from cache should contain first hops."
        );
        Ok(())
    }

    fn make_cp_partial_window() -> ContactPlan<NoManagement, NoManagement, EVLManager> {
        ContactPlan::new(
            vec![
                make_node(0, "source", NoManagement {}),
                make_node(1, "relay", NoManagement {}),
                make_node(2, "dest_a", NoManagement {}),
                make_node(3, "relay2", NoManagement {}),
                make_node(4, "dest_b", NoManagement {}),
            ],
            vec![
                make_contact::<NoManagement>(0, 2, 0.0, 2000.0, 100.0, 0.0),
                make_contact::<NoManagement>(0, 1, 0.0, 2000.0, 100.0, 0.0),
                make_contact::<NoManagement>(1, 3, 0.0, 2000.0, 100.0, 0.0),
                make_contact::<NoManagement>(3, 4, 0.0, 5.0, 100.0, 0.0),
            ],
            None,
        )
        .expect("TEST SETUP FAILED: Failed to create M3 ContactPlan.")
    }

    #[test]
    fn test_route_multicast_cache_hit_partial() -> Result<(), ASABRError> {
        let cache = Rc::new(RefCell::new(TreeCache::new(false, false, 10)));
        let mut spsn = Spsn::<
            NoManagement,
            EVLManager,
            NodeParentingTreeExcl<NoManagement, EVLManager, Hop>,
            TreeCache<NoManagement, EVLManager>,
        >::new(make_cp_partial_window(), cache, false)?;

        let bundle = make_multicast_bundle(vec![2, 4], 1, 1.0, 1000.0);
        let result1 = spsn.route(0, &bundle, 0.0, &[])?;

        assert!(
            result1.is_some(),
            "TEST FAILED: First multicast at t = 0.0 should succeed."
        );
        let result2 = spsn.route(0, &bundle, 6.0, &[])?;
        assert!(
            result2.is_some(),
            "TEST FAILED: Partial cache hit should still return a result for reachable destinations."
        );
        assert!(
            !result2.unwrap().first_hops.is_empty(),
            "TEST FAILED: RoutingOutput should contain first hops for reachable destinations (dest 2 only)."
        );
        Ok(())
    }

    #[test]
    fn test_guard_with_priorities_isolates_by_priority() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(true);
        let bundle_p1 = make_bundle(4, 1, 5.0, 1000.0);
        spsn.route(0, &bundle_p1, 0.0, &[1, 2, 3])?;
        let bundle_p2 = make_bundle(4, 2, 1.0, 1000.0);
        let result = spsn.route(0, &bundle_p2, 0.0, &[])?;

        assert!(
            result.is_some(),
            "TEST FAILED: priority 2 should not be blocked by priority 1 limit."
        );
        Ok(())
    }

    #[test]
    fn test_guard_without_priorities_ignores_priority() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(false);
        let bundle_p1 = make_bundle(4, 1, 5.0, 1000.0);
        spsn.route(0, &bundle_p1, 0.0, &[1, 2, 3])?;
        let bundle_p2 = make_bundle(4, 2, 1.0, 1000.0);
        let result = spsn.route(0, &bundle_p2, 0.0, &[])?;

        assert!(
            result.is_none(),
            "TEST FAILED: priority 2 should be blocked by priority 1 limit."
        );
        Ok(())
    }

    #[test]
    fn test_guard_add_limit_ignores_larger_value() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(false);
        let bundle_small = make_bundle(4, 1, 3.0, 1000.0);
        spsn.route(0, &bundle_small, 0.0, &[1, 2, 3])?;
        let bundle_large = make_bundle(4, 1, 5.0, 1000.0);
        spsn.route(0, &bundle_large, 0.0, &[1, 2, 3])?;
        let bundle_valid = make_bundle(4, 1, 4.0, 1000.0);
        let result = spsn.route(0, &bundle_valid, 0.0, &[])?;

        assert!(
            result.is_some(),
            "TEST FAILED: Guard limit should stay at 3.0, bundle of size 4.0 should not be blocked"
        );
        Ok(())
    }

    #[test]
    fn test_route_multicast_no_reachable_destinations() -> Result<(), ASABRError> {
        let mut spsn = make_spsn(false);
        let bundle = make_multicast_bundle(vec![2, 4], 1, 1.0, 1000.0);
        let result = spsn.route(0, &bundle, 0.0, &[1, 2, 3])?;

        assert!(
            result.is_some(),
            "TEST FAILED: route_multicast always returns Some even with 0 reachable destinations"
        );
        assert!(
            result.unwrap().first_hops.is_empty(),
            "TEST FAILED: RoutingOutput should have empty first_hops when no destination is reachable"
        );
        Ok(())
    }
}
