//! Tests for ScanService.

use tokio::sync::mpsc::error::TryRecvError;
use tower::{Service, ServiceExt};

use color_eyre::{eyre::eyre, Result};

use zebra_chain::{block::Height, parameters::Network};
use zebra_node_services::scan_service::{request::Request, response::Response};
use zebra_state::TransactionIndex;

use crate::{
    service::{scan_task::ScanTaskCommand, ScanService},
    storage::db::tests::{fake_sapling_results, new_test_storage},
    tests::ZECPAGES_SAPLING_VIEWING_KEY,
};

/// Tests that keys are deleted correctly
#[tokio::test]
pub async fn scan_service_deletes_keys_correctly() -> Result<()> {
    let mut db = new_test_storage(Network::Mainnet);

    let zec_pages_sapling_efvk = ZECPAGES_SAPLING_VIEWING_KEY.to_string();

    for fake_result_height in [Height::MIN, Height(1), Height::MAX] {
        db.insert_sapling_results(
            &zec_pages_sapling_efvk,
            fake_result_height,
            fake_sapling_results([
                TransactionIndex::MIN,
                TransactionIndex::from_index(40),
                TransactionIndex::MAX,
            ]),
        );
    }

    assert!(
        !db.sapling_results(&zec_pages_sapling_efvk).is_empty(),
        "there should be some results for this key in the db"
    );

    let (mut scan_service, mut cmd_receiver) = ScanService::new_with_mock_scanner(db);

    let response_fut = scan_service
        .ready()
        .await
        .map_err(|err| eyre!(err))?
        .call(Request::DeleteKeys(vec![zec_pages_sapling_efvk.clone()]));

    let expected_keys = vec![zec_pages_sapling_efvk.clone()];
    let cmd_handler_fut = tokio::spawn(async move {
        let Some(ScanTaskCommand::RemoveKeys { done_tx, keys }) = cmd_receiver.recv().await else {
            panic!("should successfully receive RemoveKeys message");
        };

        assert_eq!(keys, expected_keys, "keys should match the request keys");

        done_tx.send(()).expect("send should succeed");
    });

    // Poll futures
    let (response, join_result) = tokio::join!(response_fut, cmd_handler_fut);
    join_result?;

    match response.map_err(|err| eyre!(err))? {
        Response::DeletedKeys => {}
        _ => panic!("scan service returned unexpected response variant"),
    };

    assert!(
        scan_service
            .db
            .sapling_results(&zec_pages_sapling_efvk)
            .is_empty(),
        "all results for this key should have been deleted"
    );

    Ok(())
}

/// Tests that keys are deleted correctly
#[tokio::test]
pub async fn scan_service_subscribes_to_results_correctly() -> Result<()> {
    let db = new_test_storage(Network::Mainnet);

    let (mut scan_service, mut cmd_receiver) = ScanService::new_with_mock_scanner(db);

    let keys = [String::from("fake key")];

    let response_fut = scan_service
        .ready()
        .await
        .map_err(|err| eyre!(err))?
        .call(Request::SubscribeResults(keys.iter().cloned().collect()));

    let expected_keys = keys.iter().cloned().collect();
    let cmd_handler_fut = tokio::spawn(async move {
        let Some(ScanTaskCommand::SubscribeResults {
            result_sender: _,
            keys,
        }) = cmd_receiver.recv().await
        else {
            panic!("should successfully receive SubscribeResults message");
        };

        assert_eq!(keys, expected_keys, "keys should match the request keys");
    });

    // Poll futures
    let (response, join_result) = tokio::join!(response_fut, cmd_handler_fut);
    join_result?;

    let mut results_receiver = match response.map_err(|err| eyre!(err))? {
        Response::SubscribeResults(results_receiver) => results_receiver,
        _ => panic!("scan service returned unexpected response variant"),
    };

    assert_eq!(
        results_receiver.try_recv(),
        Err(TryRecvError::Disconnected),
        "channel with no items and dropped sender should be closed"
    );

    Ok(())
}

/// Tests that results are cleared are deleted correctly
#[tokio::test]
pub async fn scan_service_clears_results_correctly() -> Result<()> {
    let mut db = new_test_storage(Network::Mainnet);

    let zec_pages_sapling_efvk = ZECPAGES_SAPLING_VIEWING_KEY.to_string();

    for fake_result_height in [Height::MIN, Height(1), Height::MAX] {
        db.insert_sapling_results(
            &zec_pages_sapling_efvk,
            fake_result_height,
            fake_sapling_results([
                TransactionIndex::MIN,
                TransactionIndex::from_index(40),
                TransactionIndex::MAX,
            ]),
        );
    }

    assert!(
        !db.sapling_results(&zec_pages_sapling_efvk).is_empty(),
        "there should be some results for this key in the db"
    );

    let (mut scan_service, _cmd_receiver) = ScanService::new_with_mock_scanner(db.clone());

    let response = scan_service
        .ready()
        .await
        .map_err(|err| eyre!(err))?
        .call(Request::ClearResults(vec![zec_pages_sapling_efvk.clone()]))
        .await
        .map_err(|err| eyre!(err))?;

    match response {
        Response::ClearedResults => {}
        _ => panic!("scan service returned unexpected response variant"),
    };

    assert_eq!(
        db.sapling_results(&zec_pages_sapling_efvk).len(),
        1,
        "all results for this key should have been deleted, one empty entry should remain"
    );

    for (_, result) in db.sapling_results(&zec_pages_sapling_efvk) {
        assert!(
            result.is_empty(),
            "there should be no results for this entry in the db"
        );
    }

    Ok(())
}

/// Tests that results for key are returned correctly
#[tokio::test]
pub async fn scan_service_get_results_for_key_correctly() -> Result<()> {
    let mut db = new_test_storage(Network::Mainnet);

    let zec_pages_sapling_efvk = ZECPAGES_SAPLING_VIEWING_KEY.to_string();

    for fake_result_height in [Height::MIN, Height(1), Height::MAX] {
        db.insert_sapling_results(
            &zec_pages_sapling_efvk,
            fake_result_height,
            fake_sapling_results([
                TransactionIndex::MIN,
                TransactionIndex::from_index(40),
                TransactionIndex::MAX,
            ]),
        );
    }

    assert!(
        db.sapling_results(&zec_pages_sapling_efvk).len() == 3,
        "there should be 3 heights for this key in the db"
    );

    for (_height, transactions) in db.sapling_results(&zec_pages_sapling_efvk) {
        assert!(
            transactions.len() == 3,
            "there should be 3 transactions for each height for this key in the db"
        );
    }

    // We don't need to send any command to the scanner for this call.
    let (mut scan_service, _cmd_receiver) = ScanService::new_with_mock_scanner(db);

    let response_fut = scan_service
        .ready()
        .await
        .map_err(|err| eyre!(err))?
        .call(Request::Results(vec![zec_pages_sapling_efvk.clone()]));

    match response_fut.await.map_err(|err| eyre!(err))? {
        Response::Results(results) => {
            assert!(
                results.contains_key(&zec_pages_sapling_efvk),
                "results should contain the requested key"
            );
            assert!(results.len() == 1, "values are only for 1 key");

            assert!(
                results
                    .get_key_value(&zec_pages_sapling_efvk)
                    .unwrap()
                    .1
                    .len()
                    == 3,
                "we should have 3 heights for the given key "
            );

            for transactions in results
                .get_key_value(&zec_pages_sapling_efvk)
                .unwrap()
                .1
                .values()
            {
                assert!(
                    transactions.len() == 3,
                    "there should be 3 transactions for each height for this key"
                );
            }
        }
        _ => panic!("scan service returned unexpected response variant"),
    };

    Ok(())
}
