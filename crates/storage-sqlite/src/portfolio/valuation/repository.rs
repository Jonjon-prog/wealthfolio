use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::sql_query;
use diesel::sql_types::Text;
use diesel::sqlite::Sqlite;
use diesel::sqlite::SqliteConnection;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;

use super::model::DailyAccountValuationDB;
use crate::db::{get_connection, WriteHandle};
use crate::errors::StorageError;
use crate::schema::daily_account_valuation;
use crate::schema::daily_account_valuation::dsl::*;
use wealthfolio_core::errors::Result;
use wealthfolio_core::portfolio::valuation::{
    DailyAccountValuation, NegativeBalanceInfo, ValuationRepositoryTrait,
};

pub struct ValuationRepository {
    pool: Arc<Pool<ConnectionManager<SqliteConnection>>>,
    writer: WriteHandle,
}

impl ValuationRepository {
    pub fn new(pool: Arc<Pool<ConnectionManager<SqliteConnection>>>, writer: WriteHandle) -> Self {
        Self { pool, writer }
    }
}

#[async_trait]
impl ValuationRepositoryTrait for ValuationRepository {
    async fn save_valuations(&self, valuation_records: &[DailyAccountValuation]) -> Result<()> {
        if valuation_records.is_empty() {
            return Ok(());
        }

        // Materialize the records once before moving into the closure
        let records_to_save: Vec<DailyAccountValuationDB> = valuation_records
            .iter()
            .cloned()
            .map(DailyAccountValuationDB::from)
            .collect();

        self.writer
            .exec(move |conn| {
                for chunk in records_to_save.chunks(1000) {
                    diesel::replace_into(daily_account_valuation::table)
                        .values(chunk) // Pass the chunk directly
                        .execute(conn)
                        .map_err(StorageError::from)?;
                }
                Ok(())
            })
            .await
    }

    fn get_historical_valuations(
        &self,
        input_account_id: &str,
        start_date_opt: Option<NaiveDate>,
        end_date_opt: Option<NaiveDate>,
    ) -> Result<Vec<DailyAccountValuation>> {
        let mut conn = get_connection(&self.pool)?;

        let mut query = daily_account_valuation::table
            .filter(account_id.eq(input_account_id))
            .order(valuation_date.asc())
            .into_boxed();

        if let Some(start_date_val) = start_date_opt {
            query = query.filter(valuation_date.ge(start_date_val));
        }

        if let Some(end_date_val) = end_date_opt {
            query = query.filter(valuation_date.le(end_date_val));
        }

        let history_dbs = query
            .load::<DailyAccountValuationDB>(&mut conn)
            .map_err(StorageError::from)?;

        // Convert Vec<DailyAccountValuationDB> to Vec<DailyAccountValuation>
        // Handle potential conversion errors if necessary (using From implicitly handles unwrap_or_default)
        let history_records: Vec<DailyAccountValuation> = history_dbs
            .into_iter()
            .map(DailyAccountValuation::from)
            .collect();

        Ok(history_records)
    }

    fn load_latest_valuation_date(&self, input_account_id: &str) -> Result<Option<NaiveDate>> {
        use diesel::OptionalExtension; // Ensure OptionalExtension is in scope
        let mut conn = get_connection(&self.pool)?;

        // Select the max date. This returns Option<NaiveDate> at the SQL level.
        // Execute with .first(). This returns Result<T, Error> where T is Option<NaiveDate>.
        // Use .optional() to convert Result<Option<NaiveDate>, Error> where Error=NotFound to Ok(None),
        // and other errors to Err(...). This yields a Result<Option<Option<NaiveDate>>, Error>.
        let result: Option<Option<NaiveDate>> = daily_account_valuation::table
            .filter(account_id.eq(input_account_id))
            .select(diesel::dsl::max(valuation_date))
            .first::<Option<NaiveDate>>(&mut conn)
            .optional()
            .map_err(StorageError::from)?;

        // Flatten the Option<Option<NaiveDate>> to Option<NaiveDate>
        let latest_date = result.flatten();

        Ok(latest_date)
    }

    async fn delete_valuations_for_account(
        &self,
        input_account_id: &str,
        since_date: Option<NaiveDate>,
    ) -> Result<()> {
        let account_id_owned = input_account_id.to_string();
        self.writer
            .exec(move |conn| {
                match since_date {
                    None => {
                        diesel::delete(
                            daily_account_valuation::table.filter(account_id.eq(account_id_owned)),
                        )
                        .execute(conn)
                        .map_err(StorageError::from)?;
                    }
                    Some(date) => {
                        let date_str = date.to_string();
                        diesel::delete(
                            daily_account_valuation::table
                                .filter(account_id.eq(account_id_owned))
                                .filter(valuation_date.ge(date_str)),
                        )
                        .execute(conn)
                        .map_err(StorageError::from)?;
                    }
                }
                Ok(())
            })
            .await
    }

    fn get_latest_valuations(
        &self,
        input_account_ids: &[String],
    ) -> Result<Vec<DailyAccountValuation>> {
        if input_account_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut conn = get_connection(&self.pool)?;

        let placeholders: String = input_account_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<&str>>()
            .join(", ");

        // Ensure all fields from DailyAccountValuationDB are selected, in the correct order.
        let sql = format!(
            "WITH RankedValuations AS ( \
                SELECT \
                    id, account_id, valuation_date, account_currency, base_currency, \
                    fx_rate_to_base, cash_balance, investment_market_value, total_value, \
                    cost_basis, net_contribution, calculated_at, \
                    ROW_NUMBER() OVER (PARTITION BY account_id ORDER BY valuation_date DESC) as rn \
                FROM {} \
                WHERE account_id IN ({}) \
            ) \
            SELECT \
                id, account_id, valuation_date, account_currency, base_currency, \
                fx_rate_to_base, cash_balance, investment_market_value, total_value, \
                cost_basis, net_contribution, calculated_at \
            FROM RankedValuations \
            WHERE rn = 1",
            "daily_account_valuation", // Use direct table name string
            placeholders
        );

        let mut query_builder = sql_query(sql).into_boxed::<Sqlite>();

        for acc_id_str in input_account_ids {
            query_builder = query_builder.bind::<Text, _>(acc_id_str);
        }

        let latest_valuations_db: Vec<DailyAccountValuationDB> = query_builder
            .load::<DailyAccountValuationDB>(&mut conn)
            .map_err(StorageError::from)?;

        // To maintain input order, we first put results into a map
        let mut results_map: HashMap<String, DailyAccountValuation> = latest_valuations_db
            .into_iter()
            .map(|db_item| {
                (
                    db_item.account_id.clone(),
                    DailyAccountValuation::from(db_item),
                )
            })
            .collect();

        // Then build the ordered Vec
        let mut ordered_results = Vec::new();
        for acc_id_str in input_account_ids {
            if let Some(valuation) = results_map.remove(acc_id_str) {
                // Use remove to avoid cloning if DailyAccountValuation is large
                ordered_results.push(valuation);
            }
        }
        Ok(ordered_results)
    }

    fn get_accounts_with_negative_balance(
        &self,
        input_account_ids: &[String],
    ) -> Result<Vec<NegativeBalanceInfo>> {
        if input_account_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = get_connection(&self.pool)?;
        let placeholders: String = input_account_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<&str>>()
            .join(", ");
        // SQLite returns non-aggregated columns from the row that determines MIN().
        let sql = format!(
            "SELECT account_id, MIN(valuation_date) AS first_negative_date, \
             cash_balance, total_value, account_currency \
             FROM daily_account_valuation \
             WHERE CAST(total_value AS REAL) < 0 AND account_id IN ({}) \
             GROUP BY account_id",
            placeholders
        );
        let mut query_builder = sql_query(sql).into_boxed::<Sqlite>();
        for acc_id in input_account_ids {
            query_builder = query_builder.bind::<Text, _>(acc_id);
        }
        #[derive(QueryableByName)]
        struct NegativeBalanceRow {
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "account_id")]
            acc_id: String,
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "first_negative_date")]
            neg_date: String,
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "cash_balance")]
            cash_bal: String,
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "total_value")]
            total_val: String,
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "account_currency")]
            acc_currency: String,
        }
        let rows: Vec<NegativeBalanceRow> = query_builder
            .load::<NegativeBalanceRow>(&mut conn)
            .map_err(StorageError::from)?;
        let result = rows
            .into_iter()
            .filter_map(|r| {
                let date = NaiveDate::parse_from_str(&r.neg_date, "%Y-%m-%d").ok()?;
                let cash = r.cash_bal.parse::<rust_decimal::Decimal>().ok()?;
                let total = r.total_val.parse::<rust_decimal::Decimal>().ok()?;
                Some(NegativeBalanceInfo {
                    account_id: r.acc_id,
                    first_negative_date: date,
                    cash_balance: cash,
                    total_value: total,
                    account_currency: r.acc_currency,
                })
            })
            .collect();
        Ok(result)
    }

    fn get_multi_account_historical_valuations(
        &self,
        account_ids: &[&str],
        composite_id: &str,
        start_date: Option<NaiveDate>,
        end_date: Option<NaiveDate>,
    ) -> Result<Vec<DailyAccountValuation>> {
        if account_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut conn = get_connection(&self.pool)?;

        let placeholders = account_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let mut sql = format!(
            "SELECT valuation_date,
                    MAX(base_currency) AS base_currency,
                    SUM(CAST(cash_balance AS REAL)) AS cash_balance,
                    SUM(CAST(investment_market_value AS REAL)) AS investment_market_value,
                    SUM(CAST(total_value AS REAL)) AS total_value,
                    SUM(CAST(cost_basis AS REAL)) AS cost_basis,
                    SUM(CAST(net_contribution AS REAL)) AS net_contribution,
                    MAX(calculated_at) AS calculated_at
             FROM daily_account_valuation
             WHERE account_id IN ({placeholders})"
        );
        if start_date.is_some() {
            sql.push_str(" AND valuation_date >= ?");
        }
        if end_date.is_some() {
            sql.push_str(" AND valuation_date <= ?");
        }
        sql.push_str(" GROUP BY valuation_date ORDER BY valuation_date ASC");

        #[derive(QueryableByName)]
        struct MultiRow {
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "valuation_date")]
            val_date: String,
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "base_currency")]
            base_cur: String,
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "cash_balance")]
            cash_bal: String,
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "investment_market_value")]
            inv_val: String,
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "total_value")]
            total_val: String,
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "cost_basis")]
            cost_bas: String,
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "net_contribution")]
            net_contrib: String,
            #[diesel(sql_type = diesel::sql_types::Text, column_name = "calculated_at")]
            calc_at: String,
        }

        let mut q = sql_query(&sql).into_boxed::<Sqlite>();
        for acc_id in account_ids {
            q = q.bind::<Text, _>(*acc_id);
        }
        if let Some(s) = start_date {
            q = q.bind::<Text, _>(s.to_string());
        }
        if let Some(e) = end_date {
            q = q.bind::<Text, _>(e.to_string());
        }

        let rows: Vec<MultiRow> = q.load(&mut conn).map_err(StorageError::from)?;

        let results = rows
            .into_iter()
            .filter_map(|r| {
                let date = NaiveDate::parse_from_str(&r.val_date, "%Y-%m-%d").ok()?;
                Some(DailyAccountValuation {
                    id: format!("{composite_id}-{}", r.val_date),
                    account_id: composite_id.to_string(),
                    valuation_date: date,
                    account_currency: r.base_cur.clone(),
                    base_currency: r.base_cur,
                    fx_rate_to_base: Decimal::ONE,
                    cash_balance: r.cash_bal.parse().unwrap_or_default(),
                    investment_market_value: r.inv_val.parse().unwrap_or_default(),
                    total_value: r.total_val.parse().unwrap_or_default(),
                    cost_basis: r.cost_bas.parse().unwrap_or_default(),
                    net_contribution: r.net_contrib.parse().unwrap_or_default(),
                    calculated_at: DateTime::parse_from_rfc3339(&r.calc_at)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .collect();

        Ok(results)
    }

    fn get_valuations_on_date(
        &self,
        input_account_ids: &[String],
        input_date: NaiveDate,
    ) -> Result<Vec<DailyAccountValuation>> {
        if input_account_ids.is_empty() {
            return Ok(Vec::new()); // No need to query if the list is empty
        }

        let mut conn = get_connection(&self.pool)?;

        let history_dbs = daily_account_valuation::table
            .filter(account_id.eq_any(input_account_ids)) // Use eq_any for multiple IDs
            .filter(valuation_date.eq(input_date)) // Filter by the specific date
            .load::<DailyAccountValuationDB>(&mut conn)
            .map_err(StorageError::from)?;

        // Convert Vec<DailyAccountValuationDB> to Vec<DailyAccountValuation>
        let history_records: Vec<DailyAccountValuation> = history_dbs
            .into_iter()
            .map(DailyAccountValuation::from)
            .collect();

        Ok(history_records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{create_pool, run_migrations, write_actor::spawn_writer};
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use tempfile::tempdir;
    use wealthfolio_core::portfolio::valuation::ValuationRepositoryTrait;

    async fn setup() -> (
        ValuationRepository,
        Arc<Pool<ConnectionManager<SqliteConnection>>>,
        tempfile::TempDir,
    ) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db").to_string_lossy().to_string();
        run_migrations(&db_path).unwrap();
        let pool = create_pool(&db_path).unwrap();
        let writer = spawn_writer((*pool).clone()).unwrap();
        let repo = ValuationRepository::new(Arc::clone(&pool), writer);
        (repo, pool, dir)
    }

    fn insert_valuation(
        pool: &Arc<Pool<ConnectionManager<SqliteConnection>>>,
        acc_id: &str,
        val_date: &str,
        val_total: f64,
        val_net: f64,
    ) {
        let mut conn = get_connection(pool).unwrap();
        diesel::sql_query(format!(
            "INSERT OR IGNORE INTO accounts (id, name, account_type, currency, is_default, is_active, created_at, updated_at) \
             VALUES ('{acc_id}', 'Test', 'REGULAR', 'USD', false, true, datetime('now'), datetime('now'))"
        ))
        .execute(&mut conn)
        .unwrap();

        diesel::sql_query(format!(
            "INSERT INTO daily_account_valuation \
             (id, account_id, valuation_date, account_currency, base_currency, fx_rate_to_base, \
              cash_balance, investment_market_value, total_value, cost_basis, net_contribution, calculated_at) \
             VALUES ('{acc_id}-{val_date}', '{acc_id}', '{val_date}', 'USD', 'USD', '1.0', \
                     '0', '{val_total}', '{val_total}', '0', '{val_net}', datetime('now'))"
        ))
        .execute(&mut conn)
        .unwrap();
    }

    #[tokio::test]
    async fn test_multi_account_aggregates_by_date() {
        let (repo, pool, _dir) = setup().await;

        // Two accounts with valuations on the same dates
        insert_valuation(&pool, "acc-a", "2024-01-01", 1000.0, 500.0);
        insert_valuation(&pool, "acc-b", "2024-01-01", 2000.0, 800.0);
        insert_valuation(&pool, "acc-a", "2024-01-02", 1100.0, 500.0);
        insert_valuation(&pool, "acc-b", "2024-01-02", 2100.0, 800.0);

        let result = repo
            .get_multi_account_historical_valuations(
                &["acc-a", "acc-b"],
                "MULTI:acc-a,acc-b",
                None,
                None,
            )
            .unwrap();

        assert_eq!(result.len(), 2, "should have one row per date");

        let day1 = &result[0];
        assert_eq!(
            day1.valuation_date,
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
        );
        assert_eq!(day1.total_value, Decimal::from(3000));
        assert_eq!(day1.net_contribution, Decimal::from(1300));
        assert_eq!(day1.account_id, "MULTI:acc-a,acc-b");

        let day2 = &result[1];
        assert_eq!(day2.total_value, Decimal::from(3200));
    }

    #[tokio::test]
    async fn test_multi_account_missing_date_for_one_account() {
        let (repo, pool, _dir) = setup().await;

        // acc-a starts before acc-b
        insert_valuation(&pool, "acc-a", "2024-01-01", 1000.0, 500.0);
        insert_valuation(&pool, "acc-a", "2024-01-02", 1100.0, 500.0);
        insert_valuation(&pool, "acc-b", "2024-01-02", 2000.0, 800.0); // no acc-b on day 1

        let result = repo
            .get_multi_account_historical_valuations(
                &["acc-a", "acc-b"],
                "MULTI:acc-a,acc-b",
                None,
                None,
            )
            .unwrap();

        assert_eq!(result.len(), 2);
        // Day 1: only acc-a
        assert_eq!(result[0].total_value, Decimal::from(1000));
        // Day 2: both accounts summed
        assert_eq!(result[1].total_value, Decimal::from(3100));
    }

    #[tokio::test]
    async fn test_multi_account_date_filter() {
        let (repo, pool, _dir) = setup().await;

        insert_valuation(&pool, "acc-a", "2024-01-01", 1000.0, 500.0);
        insert_valuation(&pool, "acc-a", "2024-01-02", 1100.0, 500.0);
        insert_valuation(&pool, "acc-a", "2024-01-03", 1200.0, 500.0);

        let start = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 1, 2).unwrap();

        let result = repo
            .get_multi_account_historical_valuations(
                &["acc-a"],
                "MULTI:acc-a",
                Some(start),
                Some(end),
            )
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].total_value, Decimal::from(1100));
    }
}
