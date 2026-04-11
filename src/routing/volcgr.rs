use crate::{
    bundle::Bundle,
    contact_manager::ContactManager,
    contact_plan::ContactPlan,
    errors::ASABRError,
    multigraph::Multigraph,
    node_manager::NodeManager,
    pathfinding::Pathfinding,
    route_stage::RouteStage,
    route_storage::{Route, RouteStorage},
    types::{Date, NodeID},
};

use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use super::{Router, RoutingOutput, dry_run_unicast_path, schedule_unicast_path};

pub struct VolCgr<
    NM: NodeManager,
    CM: ContactManager,
    P: Pathfinding<NM, CM>,
    S: RouteStorage<NM, CM>,
> {
    route_storage: Rc<RefCell<S>>,
    pathfinding: P,

    // for compilation
    #[doc(hidden)]
    _phantom_nm: PhantomData<NM>,
    #[doc(hidden)]
    _phantom_cm: PhantomData<CM>,
}

impl<NM: NodeManager, CM: ContactManager, P: Pathfinding<NM, CM>, S: RouteStorage<NM, CM>>
    Router<NM, CM> for VolCgr<NM, CM, P, S>
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

        Err(ASABRError::MulticastUnsupportedError)
    }
}

impl<S: RouteStorage<NM, CM>, NM: NodeManager, CM: ContactManager, P: Pathfinding<NM, CM>>
    VolCgr<NM, CM, P, S>
{
    pub fn new(
        contact_plan: ContactPlan<NM, CM>,
        route_storage: Rc<RefCell<S>>,
    ) -> Result<Self, ASABRError> {
        Ok(Self {
            pathfinding: P::new(Rc::new(RefCell::new(Multigraph::new(contact_plan)?))),
            route_storage: route_storage.clone(),
            // for compilation
            _phantom_nm: PhantomData,
            _phantom_cm: PhantomData,
        })
    }

    fn route_unicast(
        &mut self,
        source: NodeID,
        bundle: &Bundle,
        curr_time: Date,
        excluded_nodes: &[NodeID],
    ) -> Result<Option<RoutingOutput<NM, CM>>, ASABRError> {
        let dest = bundle.destinations[0];

        let route_option = self.route_storage.try_borrow_mut()?.select(
            bundle,
            curr_time,
            self.pathfinding.get_multigraph().clone(),
            excluded_nodes,
        )?;

        if let Some(route) = route_option {
            return Ok(Some(schedule_unicast_path(
                bundle,
                curr_time,
                route.source_stage.clone(),
            )?));
        }

        let new_tree = self
            .pathfinding
            .get_next(curr_time, source, bundle, excluded_nodes)?;
        let tree = Rc::new(RefCell::new(new_tree));

        let Some(route) = Route::from_tree(tree, dest) else {
            return Ok(None);
        };
        RouteStage::init_route(route.destination_stage.clone())?;
        self.route_storage
            .try_borrow_mut()?
            .store(bundle, route.clone());
        let dry_run = dry_run_unicast_path(bundle, curr_time, route.source_stage.clone(), true)?;
        if dry_run.is_some() {
            return Ok(Some(schedule_unicast_path(
                bundle,
                curr_time,
                route.source_stage.clone(),
            )?));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_manager::legacy::evl::EVLManager;
    use crate::distance::sabr::SABR;
    use crate::node_manager::none::NoManagement;
    use crate::pathfinding::hybrid_parenting::{HybridParentingPath, HybridParentingPathExcl};
    use crate::pathfinding::test_helpers::*;
    use crate::route_storage::table::RoutingTable;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn test_volcgr_start() -> Result<(), ASABRError> {
        let cp = make_cp();
        let storage = Rc::new(RefCell::new(
            RoutingTable::<NoManagement, EVLManager, SABR>::new(),
        ));
        let mut router = VolCgr::<
            NoManagement,
            EVLManager,
            HybridParentingPath<NoManagement, EVLManager, SABR>,
            RoutingTable<NoManagement, EVLManager, SABR>,
        >::new(cp, storage.clone())?;

        // First routage
        let bundle = make_bundle(2, 1, 1.0, 1000.0);
        let output1 = router
            .route(0, &bundle, 0.0, &[])?
            .expect("First routing should succeed");

        {
            let table_borrow = storage.borrow();
            assert!(
                table_borrow.has_route_to(2),
                "Route should be stored for node 2"
            );
        }
        //Free RefCell::borrow

        // Second routage
        let output2 = router
            .route(0, &bundle, 0.0, &[])?
            .expect("Second routing should succeed (from cache)");

        assert_eq!(output1.first_hops.len(), output2.first_hops.len());
        {
            let table = storage.borrow();
            assert_eq!(table.route_count_for(2), 1, "Duplicate detected in cache");
        }
        //Free RefCell::borrow
        Ok(())
    }

    #[test]
    fn test_volcgr_consummed() -> Result<(), ASABRError> {
        let cp = make_cp();
        let storage = Rc::new(RefCell::new(
            RoutingTable::<NoManagement, EVLManager, SABR>::new(),
        ));
        let mut router = VolCgr::<
            NoManagement,
            EVLManager,
            HybridParentingPath<NoManagement, EVLManager, SABR>,
            RoutingTable<NoManagement, EVLManager, SABR>,
        >::new(cp, storage.clone())?;

        // First routage
        let bundle = make_bundle(2, 1, 10.0, 1000.0);
        let output1 = router
            .route(0, &bundle, 0.0, &[])?
            .expect("First routing should succeed");

        {
            let table_borrow = storage.borrow();
            assert!(
                table_borrow.has_route_to(2),
                "Route should be stored for node 2"
            );
        }
        //Free RefCell::borrow

        // Second routage
        let output2 = router
            .route(0, &bundle, 0.0, &[])?
            .expect("Second routing should succeed");

        let hop1 = output1
            .lazy_get_for_unicast(2)
            .expect("Should have route to 2");
        let hop2 = output2
            .lazy_get_for_unicast(2)
            .expect("Should have route to 2");

        assert_eq!(
            hop1.1.borrow().hop_count,
            2,
            "First route should be 2 hops (0->1->2)"
        );
        assert_eq!(
            hop2.1.borrow().hop_count,
            1,
            "Second route should be 1 hop (0->2 direct)"
        );
        {
            let table = storage.borrow();
            assert_eq!(
                table.route_count_for(2),
                2,
                "New Road created : contact 1 is depleted"
            );
        }
        //Free RefCell::borrow
        Ok(())
    }

    #[test]
    fn test_volcgr_excluded() -> Result<(), ASABRError> {
        let cp = make_cp();
        let storage = Rc::new(RefCell::new(
            RoutingTable::<NoManagement, EVLManager, SABR>::new(),
        ));
        let mut router = VolCgr::<
            NoManagement,
            EVLManager,
            HybridParentingPathExcl<NoManagement, EVLManager, SABR>,
            RoutingTable<NoManagement, EVLManager, SABR>,
        >::new(cp, storage.clone())?;

        let bundle = make_bundle(2, 1, 1.0, 1000.0);

        let output1 = router
            .route(0, &bundle, 0.0, &[])?
            .expect("First routing should succeed");
        assert_eq!(
            output1
                .lazy_get_for_unicast(2)
                .unwrap()
                .1
                .borrow()
                .hop_count,
            2,
            "First route should be 2 hops (0->1->2)"
        );

        let output2 = router
            .route(0, &bundle, 0.0, &[1])?
            .expect("Second routing should succeed via alternate path");
        assert_eq!(
            output2
                .lazy_get_for_unicast(2)
                .unwrap()
                .1
                .borrow()
                .hop_count,
            1,
            "Second route should be 1 hop (0->2 direct) since node 1 is excluded"
        );

        Ok(())
    }
}
