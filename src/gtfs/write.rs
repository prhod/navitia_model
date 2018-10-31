// Copyright 2017-2018 Kisio Digital and/or its affiliates.
//
// This program is free software: you can redistribute it and/or
// modify it under the terms of the GNU General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful, but
// WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
// General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see
// <http://www.gnu.org/licenses/>.

use super::{Agency, DirectionType, Shape, Stop, StopLocationType, StopTime, Transfer, Trip};
use collection::{Collection, CollectionWithId, Id};
use common_format::Availability;
use csv;
use failure::ResultExt;
use geo_types::Geometry as GeoGeometry;
use objects;
use objects::Transfer as NtfsTransfer;
use objects::*;
use std::path;
use Result;

pub fn write_transfers(path: &path::Path, transfers: &Collection<NtfsTransfer>) -> Result<()> {
    if transfers.is_empty() {
        return Ok(());
    }
    info!("Writing transfers.txt");
    let path = path.join("transfers.txt");
    let mut wtr = csv::Writer::from_path(&path).with_context(ctx_from_path!(path))?;
    for t in transfers.values() {
        wtr.serialize(Transfer::from(t))
            .with_context(ctx_from_path!(path))?;
    }

    wtr.flush().with_context(ctx_from_path!(path))?;

    Ok(())
}

pub fn write_agencies(
    path: &path::Path,
    networks: &CollectionWithId<objects::Network>,
) -> Result<()> {
    info!("Writing agency.txt");
    let path = path.join("agency.txt");
    let mut wtr = csv::Writer::from_path(&path).with_context(ctx_from_path!(path))?;
    for n in networks.values() {
        wtr.serialize(Agency::from(n))
            .with_context(ctx_from_path!(path))?;
    }

    wtr.flush().with_context(ctx_from_path!(path))?;

    Ok(())
}

/// get the first comment ordered by name
fn get_first_comment_name<T: objects::CommentLinks>(
    obj: &T,
    comments: &CollectionWithId<objects::Comment>,
) -> String {
    comments
        .iter_from(obj.comment_links())
        .map(|c| &c.name)
        .min()
        .cloned()
        .unwrap_or_else(|| "".into())
}

fn ntfs_stop_point_to_gtfs_stop(
    sp: &objects::StopPoint,
    comments: &CollectionWithId<objects::Comment>,
    equipments: &CollectionWithId<objects::Equipment>,
) -> Stop {
    let wheelchair = sp
        .equipment_id
        .clone()
        .and_then(|eq_id| equipments.get(&eq_id))
        .map(|eq| eq.wheelchair_boarding)
        .unwrap_or_else(|| Availability::default());
    Stop {
        id: sp.id.clone(),
        name: sp.name.clone(),
        lat: sp.coord.lat,
        lon: sp.coord.lon,
        fare_zone_id: sp.fare_zone_id.clone(),
        location_type: StopLocationType::StopPoint,
        parent_station: Some(sp.stop_area_id.clone()),
        code: None,
        desc: get_first_comment_name(sp, comments),
        wheelchair_boarding: wheelchair,
        url: None,
        timezone: sp.timezone.clone(),
    }
}

fn ntfs_stop_area_to_gtfs_stop(
    sa: &objects::StopArea,
    comments: &CollectionWithId<objects::Comment>,
    equipments: &CollectionWithId<objects::Equipment>,
) -> Stop {
    let wheelchair = sa
        .equipment_id
        .clone()
        .and_then(|eq_id| equipments.get(&eq_id))
        .map(|eq| eq.wheelchair_boarding)
        .unwrap_or_else(|| Availability::default());
    Stop {
        id: sa.id.clone(),
        name: sa.name.clone(),
        lat: sa.coord.lat,
        lon: sa.coord.lon,
        fare_zone_id: None,
        location_type: StopLocationType::StopArea,
        parent_station: None,
        code: None,
        desc: get_first_comment_name(sa, comments),
        wheelchair_boarding: wheelchair,
        url: None,
        timezone: sa.timezone.clone(),
    }
}

pub fn write_stops(
    path: &path::Path,
    stop_points: &CollectionWithId<objects::StopPoint>,
    stop_areas: &CollectionWithId<objects::StopArea>,
    comments: &CollectionWithId<objects::Comment>,
    equipments: &CollectionWithId<objects::Equipment>,
) -> Result<()> {
    info!("Writing stops.txt");
    let path = path.join("stops.txt");
    let mut wtr = csv::Writer::from_path(&path).with_context(ctx_from_path!(path))?;
    for sp in stop_points.values() {
        wtr.serialize(ntfs_stop_point_to_gtfs_stop(sp, comments, equipments))
            .with_context(ctx_from_path!(path))?;
    }
    for sa in stop_areas.values() {
        wtr.serialize(ntfs_stop_area_to_gtfs_stop(sa, comments, equipments))
            .with_context(ctx_from_path!(path))?;
    }

    wtr.flush().with_context(ctx_from_path!(path))?;

    Ok(())
}

fn get_gtfs_trip_shortname_and_headsign_from_ntfs_vj(
    vj: &objects::VehicleJourney,
    sps: &CollectionWithId<objects::StopPoint>,
) -> (Option<String>, Option<String>) {
    fn get_last_stop_name(
        vj: &objects::VehicleJourney,
        sps: &CollectionWithId<objects::StopPoint>,
    ) -> Option<String> {
        vj.stop_times
            .last()
            .map(|st| &sps[st.stop_point_idx].name)
            .cloned()
    }

    match vj.physical_mode_id.as_str() {
        "LocalTrain" | "LongDistanceTrain" | "Metro" | "RapidTransit" | "Train" => {
            (vj.headsign.clone(), get_last_stop_name(vj, sps))
        }
        _ => (
            None,
            vj.headsign.clone().or_else(|| get_last_stop_name(vj, sps)),
        ),
    }
}

fn get_gtfs_direction_id_from_ntfs_vj(
    vj: &objects::VehicleJourney,
    routes: &CollectionWithId<objects::Route>,
) -> DirectionType {
    let route = routes.get(&vj.route_id).unwrap();
    match route.direction_type.as_ref().map(|s| s.as_str()) {
        Some("forward") | Some("clockwise") | Some("inbound") => DirectionType::Forward,
        _ => DirectionType::Backward,
    }
}

fn make_gtfs_trip_from_ntfs_vj(
    vj: &objects::VehicleJourney,
    sps: &CollectionWithId<objects::StopPoint>,
    routes: &CollectionWithId<objects::Route>,
    tps: &CollectionWithId<objects::TripProperty>,
) -> Trip {
    let (short_name, headsign) = get_gtfs_trip_shortname_and_headsign_from_ntfs_vj(vj, sps);
    let mut wheelchair_and_bike = (Availability::default(), Availability::default());
    if let Some(tp_id) = &vj.trip_property_id {
        if let Some(tp) = tps.get(&tp_id) {
            wheelchair_and_bike = (tp.wheelchair_accessible, tp.bike_accepted);
        };
    }

    Trip {
        route_id: vj.route_id.clone(),
        service_id: vj.service_id.clone(),
        id: vj.id.clone(),
        headsign,
        short_name,
        direction: get_gtfs_direction_id_from_ntfs_vj(vj, routes),
        block_id: vj.block_id.clone(),
        shape_id: vj.geometry_id.clone(),
        wheelchair_accessible: wheelchair_and_bike.0,
        bikes_allowed: wheelchair_and_bike.1,
    }
}

pub fn write_trips(
    path: &path::Path,
    vjs: &CollectionWithId<objects::VehicleJourney>,
    sps: &CollectionWithId<objects::StopPoint>,
    routes: &CollectionWithId<objects::Route>,
    tps: &CollectionWithId<objects::TripProperty>,
) -> Result<()> {
    info!("Writing trips.txt");
    let path = path.join("trips.txt");
    let mut wtr = csv::Writer::from_path(&path).with_context(ctx_from_path!(path))?;
    for vj in vjs.values() {
        wtr.serialize(make_gtfs_trip_from_ntfs_vj(vj, sps, routes, tps))
            .with_context(ctx_from_path!(path))?;
    }

    wtr.flush().with_context(ctx_from_path!(path))?;

    Ok(())
}
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct StopExtension {
    #[serde(rename = "stop_id")]
    id: String,
    #[serde(rename = "system_name")]
    name: String,
    #[serde(rename = "system_code")]
    code: String,
}

fn stop_extensions_from_collection_with_id<'a, T>(
    collections: &'a CollectionWithId<T>,
) -> impl Iterator<Item = StopExtension> + 'a
where
    T: Id<T> + Codes,
{
    collections
        .values()
        .flat_map(|obj| obj.codes().iter().map(move |c| (obj.id(), c)))
        .map(|(id, (name, code))| StopExtension {
            id: id.to_string(),
            name: name.to_string(),
            code: code.to_string(),
        })
}

pub fn write_stop_extensions(
    path: &path::Path,
    stop_points: &CollectionWithId<StopPoint>,
    stop_areas: &CollectionWithId<StopArea>,
) -> Result<()> {
    let mut stop_extensions = Vec::new();
    stop_extensions.extend(stop_extensions_from_collection_with_id(&stop_points));
    stop_extensions.extend(stop_extensions_from_collection_with_id(&stop_areas));
    if stop_extensions.is_empty() {
        return Ok(());
    }
    info!("Writing stop_extensions.txt");
    let path = path.join("stop_extensions.txt");
    let mut wtr = csv::Writer::from_path(&path).with_context(ctx_from_path!(path))?;
    for se in stop_extensions {
        wtr.serialize(se).with_context(ctx_from_path!(path))?;
    }
    wtr.flush().with_context(ctx_from_path!(path))?;

    Ok(())
}

pub fn write_stop_times(
    path: &path::Path,
    vehicle_journeys: &CollectionWithId<VehicleJourney>,
    stop_points: &CollectionWithId<StopPoint>,
) -> Result<()> {
    info!("Writing stop_times.txt");
    let stop_times_path = path.join("stop_times.txt");
    let mut st_wtr =
        csv::Writer::from_path(&stop_times_path).with_context(ctx_from_path!(stop_times_path))?;
    for vj in vehicle_journeys.values() {
        for st in &vj.stop_times {
            st_wtr
                .serialize(StopTime {
                    stop_id: stop_points[st.stop_point_idx].id.clone(),
                    trip_id: vj.id.clone(),
                    stop_sequence: st.sequence,
                    arrival_time: st.arrival_time,
                    departure_time: st.departure_time,
                    pickup_type: st.pickup_type,
                    drop_off_type: st.drop_off_type,
                    local_zone_id: st.local_zone_id,
                }).with_context(ctx_from_path!(st_wtr))?;
        }
    }
    st_wtr
        .flush()
        .with_context(ctx_from_path!(stop_times_path))?;
    Ok(())
}

fn ntfs_geometry_to_gtfs_shapes<'a>(g: &'a objects::Geometry) -> impl Iterator<Item = Shape> + 'a {
    let points = match g.geometry {
        GeoGeometry::LineString(ref linestring) => &linestring.0[..],
        _ => {
            warn!(
                "Geometry {} is not exported, only LINESTRING geometries are exported",
                g.id
            );
            &[]
        }
    };

    points.iter().enumerate().map(move |(i, p)| Shape {
        id: g.id.clone(),
        lat: p.y(),
        lon: p.x(),
        sequence: i as u32,
    })
}

pub fn write_shapes(
    path: &path::Path,
    geometries: &CollectionWithId<objects::Geometry>,
) -> Result<()> {
    let shapes: Vec<_> = geometries
        .values()
        .flat_map(ntfs_geometry_to_gtfs_shapes)
        .collect();
    if !shapes.is_empty() {
        info!("Writing shapes.txt");
        let path = path.join("shapes.txt");
        let mut wtr = csv::Writer::from_path(&path).with_context(ctx_from_path!(path))?;
        wtr.flush().with_context(ctx_from_path!(path))?;
        for shape in shapes {
            wtr.serialize(shape).with_context(ctx_from_path!(path))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use collection::CollectionWithId;
    use common_format::write_calendar_dates;
    use geo_types::{Geometry as GeoGeometry, LineString, Point};
    use gtfs::{StopLocationType, Transfer, TransferType};
    use objects::Transfer as NtfsTransfer;
    use objects::{Calendar, CommentLinksT, Coord, KeysValues, StopPoint, StopTime};
    use std::collections::BTreeSet;
    extern crate tempdir;
    use self::tempdir::TempDir;
    use chrono;
    use std::fs::File;
    use std::io::Read;

    #[test]
    fn write_agency() {
        let agency = Agency::from(&objects::Network {
            id: "OIF:101".to_string(),
            name: "SAVAC".to_string(),
            url: Some("http://www.vianavigo.com,Europe/Paris".to_string()),
            timezone: Some("Europe/Madrid".to_string()),
            lang: Some("fr".to_string()),
            phone: Some("0123456789".to_string()),
            address: Some("somewhere".to_string()),
            sort_order: Some(1),
            codes: Default::default(),
        });

        let expected_agency = Agency {
            id: Some("OIF:101".to_string()),
            name: "SAVAC".to_string(),
            url: "http://www.vianavigo.com,Europe/Paris".to_string(),
            timezone: "Europe/Madrid".to_string(),
            lang: Some("fr".to_string()),
            phone: Some("0123456789".to_string()),
            email: None,
        };

        assert_eq!(expected_agency, agency);
    }

    #[test]
    fn write_agency_with_default_values() {
        let agency = Agency::from(&objects::Network {
            id: "OIF:101".to_string(),
            name: "SAVAC".to_string(),
            url: None,
            timezone: None,
            lang: None,
            phone: None,
            address: None,
            sort_order: None,
            codes: Default::default(),
        });

        let expected_agency = Agency {
            id: Some("OIF:101".to_string()),
            name: "SAVAC".to_string(),
            url: "http://www.navitia.io/".to_string(),
            timezone: "Europe/Paris".to_string(),
            lang: None,
            phone: None,
            email: None,
        };

        assert_eq!(expected_agency, agency);
    }

    #[test]
    fn test_ntfs_stop_point_to_gtfs_stop() {
        let comments = CollectionWithId::new(vec![
            objects::Comment {
                id: "1".into(),
                name: "foo".into(),
                comment_type: objects::CommentType::Information,
                url: None,
                label: None,
            },
            objects::Comment {
                id: "2".into(),
                name: "bar".into(),
                comment_type: objects::CommentType::Information,
                url: None,
                label: None,
            },
        ]).unwrap();

        let equipments = CollectionWithId::new(vec![objects::Equipment {
            id: "1".to_string(),
            wheelchair_boarding: Availability::Available,
            sheltered: Availability::InformationNotAvailable,
            elevator: Availability::Available,
            escalator: Availability::Available,
            bike_accepted: Availability::Available,
            bike_depot: Availability::Available,
            visual_announcement: Availability::Available,
            audible_announcement: Availability::Available,
            appropriate_escort: Availability::Available,
            appropriate_signage: Availability::Available,
        }]).unwrap();

        let mut comment_links = BTreeSet::new();
        comment_links.insert(comments.get_idx("1").unwrap());
        comment_links.insert(comments.get_idx("2").unwrap());

        let stop = objects::StopPoint {
            id: "sp_1".to_string(),
            name: "sp_name_1".to_string(),
            codes: BTreeSet::default(),
            object_properties: BTreeSet::default(),
            comment_links: comment_links,
            visible: true,
            coord: objects::Coord {
                lon: 2.073034,
                lat: 48.799115,
            },
            stop_area_id: "OIF:SA:8739322".to_string(),
            timezone: Some("Europe/Paris".to_string()),
            geometry_id: None,
            equipment_id: Some("1".to_string()),
            fare_zone_id: Some("1".to_string()),
        };

        let expected = Stop {
            id: "sp_1".to_string(),
            name: "sp_name_1".to_string(),
            lat: 48.799115,
            lon: 2.073034,
            fare_zone_id: Some("1".to_string()),
            location_type: StopLocationType::StopPoint,
            parent_station: Some("OIF:SA:8739322".to_string()),
            code: None,
            desc: "bar".to_string(),
            wheelchair_boarding: Availability::Available,
            url: None,
            timezone: Some("Europe/Paris".to_string()),
        };

        assert_eq!(
            expected,
            ntfs_stop_point_to_gtfs_stop(&stop, &comments, &equipments)
        );
    }

    #[test]
    fn test_ntfs_minimal_stop_point_to_gtfs_stop() {
        let stop = objects::StopPoint {
            id: "sp_1".to_string(),
            name: "sp_name_1".to_string(),
            codes: BTreeSet::default(),
            object_properties: BTreeSet::default(),
            comment_links: BTreeSet::default(),
            visible: true,
            coord: objects::Coord {
                lon: 2.073034,
                lat: 48.799115,
            },
            stop_area_id: "OIF:SA:8739322".to_string(),
            timezone: None,
            geometry_id: None,
            equipment_id: None,
            fare_zone_id: None,
        };

        let expected = Stop {
            id: "sp_1".to_string(),
            name: "sp_name_1".to_string(),
            lat: 48.799115,
            lon: 2.073034,
            fare_zone_id: None,
            location_type: StopLocationType::StopPoint,
            parent_station: Some("OIF:SA:8739322".to_string()),
            code: None,
            desc: "".to_string(),
            wheelchair_boarding: Availability::InformationNotAvailable,
            url: None,
            timezone: None,
        };

        let comments = CollectionWithId::default();
        let equipments = CollectionWithId::default();
        assert_eq!(
            expected,
            ntfs_stop_point_to_gtfs_stop(&stop, &comments, &equipments)
        );
    }

    #[test]
    fn test_ntfs_stop_area_to_gtfs_stop() {
        let comments = CollectionWithId::new(vec![
            objects::Comment {
                id: "1".into(),
                name: "foo".into(),
                comment_type: objects::CommentType::Information,
                url: None,
                label: None,
            },
            objects::Comment {
                id: "2".into(),
                name: "bar".into(),
                comment_type: objects::CommentType::Information,
                url: None,
                label: None,
            },
        ]).unwrap();

        let equipments = CollectionWithId::new(vec![objects::Equipment {
            id: "1".to_string(),
            wheelchair_boarding: Availability::NotAvailable,
            sheltered: Availability::InformationNotAvailable,
            elevator: Availability::Available,
            escalator: Availability::Available,
            bike_accepted: Availability::Available,
            bike_depot: Availability::Available,
            visual_announcement: Availability::Available,
            audible_announcement: Availability::Available,
            appropriate_escort: Availability::Available,
            appropriate_signage: Availability::Available,
        }]).unwrap();

        let mut comment_links = BTreeSet::new();
        comment_links.insert(comments.get_idx("1").unwrap());
        comment_links.insert(comments.get_idx("2").unwrap());

        let stop = objects::StopArea {
            id: "sa_1".to_string(),
            name: "sa_name_1".to_string(),
            codes: BTreeSet::default(),
            object_properties: BTreeSet::default(),
            comment_links: comment_links,
            visible: true,
            coord: objects::Coord {
                lon: 2.073034,
                lat: 48.799115,
            },
            timezone: Some("Europe/Paris".to_string()),
            geometry_id: None,
            equipment_id: Some("1".to_string()),
        };

        let expected = Stop {
            id: "sa_1".to_string(),
            name: "sa_name_1".to_string(),
            lat: 48.799115,
            lon: 2.073034,
            fare_zone_id: None,
            location_type: StopLocationType::StopArea,
            parent_station: None,
            code: None,
            desc: "bar".to_string(),
            wheelchair_boarding: Availability::NotAvailable,
            url: None,
            timezone: Some("Europe/Paris".to_string()),
        };

        assert_eq!(
            expected,
            ntfs_stop_area_to_gtfs_stop(&stop, &comments, &equipments)
        );
    }

    #[test]
    fn write_trip() {
        let sps = CollectionWithId::new(vec![
            objects::StopPoint {
                id: "OIF:SP:36:2085".to_string(),
                name: "Gare de Saint-Cyr l'École".to_string(),
                codes: BTreeSet::default(),
                object_properties: BTreeSet::default(),
                comment_links: BTreeSet::default(),
                visible: true,
                coord: objects::Coord {
                    lon: 2.073034,
                    lat: 48.799115,
                },
                stop_area_id: "OIF:SA:8739322".to_string(),
                timezone: Some("Europe/Paris".to_string()),
                geometry_id: None,
                equipment_id: None,
                fare_zone_id: Some("1".to_string()),
            },
            objects::StopPoint {
                id: "OIF:SP:36:2127".to_string(),
                name: "Division Leclerc".to_string(),
                codes: BTreeSet::default(),
                object_properties: BTreeSet::default(),
                comment_links: BTreeSet::default(),
                visible: true,
                coord: objects::Coord {
                    lon: 2.073407,
                    lat: 48.800598,
                },
                stop_area_id: "OIF:SA:2:1468".to_string(),
                timezone: Some("Europe/Paris".to_string()),
                geometry_id: None,
                equipment_id: None,
                fare_zone_id: None,
            },
        ]).unwrap();
        let routes = CollectionWithId::new(vec![objects::Route {
            id: "OIF:078078001:1".to_string(),
            name: "Hôtels - Hôtels".to_string(),
            direction_type: Some("forward".to_string()),
            codes: BTreeSet::default(),
            object_properties: BTreeSet::default(),
            comment_links: BTreeSet::default(),
            line_id: "OIF:002002002:BDEOIF829".to_string(),
            geometry_id: Some("Geometry:Line:Relation:6883353".to_string()),
            destination_id: Some("OIF,OIF:SA:4:126".to_string()),
        }]).unwrap();

        let tps = CollectionWithId::new(vec![objects::TripProperty {
            id: "1".to_string(),
            wheelchair_accessible: Availability::Available,
            bike_accepted: Availability::NotAvailable,
            air_conditioned: Availability::InformationNotAvailable,
            visual_announcement: Availability::Available,
            audible_announcement: Availability::Available,
            appropriate_escort: Availability::Available,
            appropriate_signage: Availability::Available,
            school_vehicle_type: objects::TransportType::Regular,
        }]).unwrap();
        let vj = objects::VehicleJourney {
            id: "OIF:87604986-1_11595-1".to_string(),
            codes: BTreeSet::default(),
            object_properties: BTreeSet::default(),
            comment_links: BTreeSet::default(),
            route_id: "OIF:078078001:1".to_string(),
            physical_mode_id: "Bus".to_string(),
            dataset_id: "OIF:0".to_string(),
            service_id: "2".to_string(),
            headsign: Some("2005".to_string()),
            block_id: Some("PLOI".to_string()),
            company_id: "OIF:743".to_string(),
            trip_property_id: Some("1".to_string()),
            geometry_id: Some("Geometry:Line:Relation:6883353".to_string()),
            stop_times: vec![
                objects::StopTime {
                    stop_point_idx: sps.get_idx("OIF:SP:36:2085").unwrap(),
                    sequence: 0,
                    arrival_time: objects::Time::new(14, 40, 0),
                    departure_time: objects::Time::new(14, 40, 0),
                    boarding_duration: 0,
                    alighting_duration: 0,
                    pickup_type: 0,
                    drop_off_type: 1,
                    datetime_estimated: false,
                    local_zone_id: None,
                },
                objects::StopTime {
                    stop_point_idx: sps.get_idx("OIF:SP:36:2127").unwrap(),
                    sequence: 1,
                    arrival_time: objects::Time::new(14, 42, 0),
                    departure_time: objects::Time::new(14, 42, 0),
                    boarding_duration: 0,
                    alighting_duration: 0,
                    pickup_type: 0,
                    drop_off_type: 0,
                    datetime_estimated: false,
                    local_zone_id: None,
                },
            ],
        };

        let expected = Trip {
            route_id: "OIF:078078001:1".to_string(),
            service_id: vj.service_id.clone(),
            id: "OIF:87604986-1_11595-1".to_string(),
            headsign: Some("2005".to_string()),
            short_name: None,
            direction: DirectionType::Forward,
            block_id: Some("PLOI".to_string()),
            shape_id: vj.geometry_id.clone(),
            wheelchair_accessible: Availability::Available,
            bikes_allowed: Availability::NotAvailable,
        };

        assert_eq!(
            expected,
            make_gtfs_trip_from_ntfs_vj(&vj, &sps, &routes, &tps)
        );
    }

    #[test]
    fn ntfs_object_code_to_stop_extensions() {
        let mut sa_codes: BTreeSet<(String, String)> = BTreeSet::new();
        sa_codes.insert(("sa name 1".to_string(), "sa_code_1".to_string()));
        sa_codes.insert(("sa name 2".to_string(), "sa_code_2".to_string()));
        let stop_areas = CollectionWithId::new(vec![StopArea {
            id: "sa:01".to_string(),
            name: "sa:01".to_string(),
            codes: sa_codes,
            object_properties: KeysValues::default(),
            comment_links: CommentLinksT::default(),
            visible: true,
            coord: Coord {
                lon: 2.073,
                lat: 48.799,
            },
            timezone: None,
            geometry_id: None,
            equipment_id: None,
        }]).unwrap();
        let mut sp_codes: BTreeSet<(String, String)> = BTreeSet::new();
        sp_codes.insert(("sp name 1".to_string(), "sp_code_1".to_string()));
        sp_codes.insert(("sp name 2".to_string(), "sp_code_2".to_string()));
        sp_codes.insert(("sp name 3".to_string(), "sp_code_3".to_string()));
        let stop_points = CollectionWithId::new(vec![StopPoint {
            id: "sp:01".to_string(),
            name: "sp:01".to_string(),
            codes: sp_codes,
            object_properties: KeysValues::default(),
            comment_links: CommentLinksT::default(),
            visible: true,
            coord: Coord {
                lon: 2.073,
                lat: 48.799,
            },
            stop_area_id: "sa:01".to_string(),
            timezone: None,
            geometry_id: None,
            equipment_id: None,
            fare_zone_id: None,
        }]).unwrap();
        let tmp_dir = TempDir::new("navitia_model_tests").expect("create temp dir");
        write_stop_extensions(tmp_dir.path(), &stop_points, &stop_areas).unwrap();
        let output_file_path = tmp_dir.path().join("stop_extensions.txt");
        let mut output_file = File::open(output_file_path.clone())
            .expect(&format!("file {:?} not found", output_file_path));
        let mut output_contents = String::new();
        output_file.read_to_string(&mut output_contents).unwrap();
        assert_eq!(
            "stop_id,system_name,system_code\n\
             sp:01,sp name 1,sp_code_1\n\
             sp:01,sp name 2,sp_code_2\n\
             sp:01,sp name 3,sp_code_3\n\
             sa:01,sa name 1,sa_code_1\n\
             sa:01,sa name 2,sa_code_2\n",
            output_contents
        );
        tmp_dir.close().expect("delete temp dir");
    }

    #[test]
    fn ntfs_object_code_to_stop_extensions_nothing_generated() {
        let stop_areas = CollectionWithId::new(vec![]).unwrap();
        let stop_points = CollectionWithId::new(vec![]).unwrap();
        let tmp_dir = TempDir::new("navitia_model_tests").expect("create temp dir");
        write_stop_extensions(tmp_dir.path(), &stop_points, &stop_areas).unwrap();
        let output_file_path = tmp_dir.path().join("stop_extensions.txt");
        assert!(!output_file_path.exists());
        tmp_dir.close().expect("delete temp dir");
    }

    #[test]
    fn ntfs_geometry_linestring_exported() {
        let geo = objects::Geometry {
            id: "1".to_string(),
            geometry: GeoGeometry::LineString(LineString(vec![
                Point::new(1.1, 2.2),
                Point::new(3.3, 4.4),
            ])),
        };

        let expected = vec![
            Shape {
                id: "1".to_string(),
                lon: 1.1,
                lat: 2.2,
                sequence: 0,
            },
            Shape {
                id: "1".to_string(),
                lon: 3.3,
                lat: 4.4,
                sequence: 1,
            },
        ];

        assert_eq!(
            expected,
            ntfs_geometry_to_gtfs_shapes(&geo).collect::<Vec<Shape>>()
        );
    }

    #[test]
    fn ntfs_geometry_not_linestring_not_exported() {
        let geo = objects::Geometry {
            id: "1".to_string(),
            geometry: GeoGeometry::Point(Point::new(1.1, 2.2)),
        };

        let shapes: Vec<Shape> = ntfs_geometry_to_gtfs_shapes(&geo).collect();

        assert!(shapes.is_empty());
    }

    #[test]
    fn ntfs_transfers_to_gtfs_transfers() {
        let transfer = Transfer::from(&NtfsTransfer {
            from_stop_id: "sp:01".to_string(),
            to_stop_id: "sp:02".to_string(),
            min_transfer_time: Some(42),
            real_min_transfer_time: None,
            equipment_id: None,
        });

        let expected = Transfer {
            from_stop_id: "sp:01".to_string(),
            to_stop_id: "sp:02".to_string(),
            transfer_type: TransferType::WithTransferTime,
            min_transfer_time: Some(42),
        };

        assert_eq!(expected, transfer);
    }

    #[test]
    fn write_calendar_dates_from_calendar() {
        let mut dates = BTreeSet::new();
        dates.insert(chrono::NaiveDate::from_ymd(2018, 5, 5));
        dates.insert(chrono::NaiveDate::from_ymd(2018, 5, 6));
        let calendar = CollectionWithId::new(vec![
            Calendar {
                id: "1".to_string(),
                dates,
            },
            Calendar {
                id: "2".to_string(),
                dates: BTreeSet::new(),
            },
        ]).unwrap();
        let tmp_dir = TempDir::new("navitia_model_tests").expect("create temp dir");
        write_calendar_dates(tmp_dir.path(), &calendar).unwrap();
        let output_file_path = tmp_dir.path().join("calendar_dates.txt");
        let mut output_file = File::open(output_file_path.clone())
            .expect(&format!("file {:?} not found", output_file_path));
        let mut output_contents = String::new();
        output_file.read_to_string(&mut output_contents).unwrap();
        assert_eq!(
            "service_id,date,exception_type\n\
             1,20180505,1\n\
             1,20180506,1\n",
            output_contents
        );
        tmp_dir.close().expect("delete temp dir");
    }

    #[test]
    fn ntfs_vehicle_journeys_to_stop_times() {
        let stop_points = CollectionWithId::new(vec![StopPoint {
            id: "sp:01".to_string(),
            name: "sp_name_1".to_string(),
            codes: KeysValues::default(),
            object_properties: KeysValues::default(),
            comment_links: CommentLinksT::default(),
            visible: true,
            coord: Coord {
                lon: 2.37,
                lat: 48.84,
            },
            timezone: None,
            geometry_id: None,
            equipment_id: None,
            stop_area_id: "sa_1".to_string(),
            fare_zone_id: None,
        }]).unwrap();
        let stop_times_vec = vec![
            StopTime {
                stop_point_idx: stop_points.get_idx("sp:01").unwrap(),
                sequence: 1,
                arrival_time: Time::new(6, 0, 0),
                departure_time: Time::new(6, 0, 0),
                boarding_duration: 0,
                alighting_duration: 0,
                pickup_type: 0,
                drop_off_type: 0,
                datetime_estimated: false,
                local_zone_id: None,
            },
            StopTime {
                stop_point_idx: stop_points.get_idx("sp:01").unwrap(),
                sequence: 2,
                arrival_time: Time::new(6, 6, 27),
                departure_time: Time::new(6, 6, 27),
                boarding_duration: 0,
                alighting_duration: 0,
                pickup_type: 2,
                drop_off_type: 1,
                datetime_estimated: false,
                local_zone_id: Some(3),
            },
        ];
        let vehicle_journeys = CollectionWithId::new(vec![VehicleJourney {
            id: "vj:01".to_string(),
            codes: BTreeSet::new(),
            object_properties: KeysValues::default(),
            comment_links: CommentLinksT::default(),
            route_id: "r:01".to_string(),
            physical_mode_id: "pm:01".to_string(),
            dataset_id: "ds:01".to_string(),
            service_id: "sv:01".to_string(),
            headsign: None,
            block_id: None,
            company_id: "c:01".to_string(),
            trip_property_id: None,
            geometry_id: None,
            stop_times: stop_times_vec,
        }]).unwrap();
        let tmp_dir = TempDir::new("navitia_model_tests").expect("create temp dir");
        write_stop_times(tmp_dir.path(), &vehicle_journeys, &stop_points).unwrap();
        let output_file_path = tmp_dir.path().join("stop_times.txt");
        let mut output_file = File::open(output_file_path.clone())
            .expect(&format!("file {:?} not found", output_file_path));
        let mut output_contents = String::new();
        output_file.read_to_string(&mut output_contents).unwrap();
        assert_eq!(
            "trip_id,arrival_time,departure_time,stop_id,stop_sequence,pickup_type,drop_off_type,local_zone_id\n\
             vj:01,06:00:00,06:00:00,sp:01,1,0,0,\n\
             vj:01,06:06:27,06:06:27,sp:01,2,2,1,3\n",
            output_contents
        );
        tmp_dir.close().expect("delete temp dir");
    }
}