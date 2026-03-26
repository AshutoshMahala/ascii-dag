//! Subgraph stress test — progressive levels to find terminal rendering limits.
//!
//! Each test adds more nodes, deeper nesting, or wider subgraphs to see
//! how much a terminal can comfortably display.
//!
//! Usage:
//!   cargo run --example subgraph_stress --release               # Heap mode
//!   cargo run --example subgraph_stress --release --features arena -- --csr  # CSR mode

use ascii_dag::graph::Graph;
use std::time::Instant;

#[cfg(feature = "arena")]
use ascii_dag::LayoutConfig;
#[cfg(feature = "arena")]
use ascii_dag::graph::arena::Arena;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let use_csr = args.iter().any(|a| a == "--csr");

    let mode = if use_csr { "CSR" } else { "Heap" };
    println!("╔══════════════════════════════════════════════════╗");
    println!("║     Subgraph Stress Test  ({:>4} mode)           ║", mode);
    println!("╚══════════════════════════════════════════════════╝\n");

    #[allow(clippy::type_complexity)]
    let tests: Vec<(&str, fn() -> Graph<'static>)> = vec![
        (
            "Tier 1 · Microservices (12 nodes, 4 subgraphs, depth 1)",
            tier1_microservices,
        ),
        (
            "Tier 2 · Platform (20 nodes, 8 subgraphs, depth 2)",
            tier2_platform,
        ),
        (
            "Tier 3 · Cloud Infra (30 nodes, 12 subgraphs, depth 3)",
            tier3_cloud,
        ),
        (
            "Tier 4 · Enterprise (50 nodes, 16 subgraphs, depth 3)",
            tier4_enterprise,
        ),
        (
            "Tier 5 · Megacorp (80 nodes, 24 subgraphs, depth 4)",
            tier5_megacorp,
        ),
    ];

    for (name, build_fn) in &tests {
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  {}", name);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

        let dag = build_fn();
        let sg_count = dag.subgraph_count();

        if use_csr {
            #[cfg(feature = "arena")]
            render_csr(&dag, sg_count);
            #[cfg(not(feature = "arena"))]
            {
                let _ = (&dag, sg_count);
                println!("  (arena feature not enabled — run with --features arena)");
            }
        } else {
            render_heap(&dag, sg_count);
        }
        println!();
    }

    println!("Done. If the last tier was still readable, your terminal is a champ.");
}

fn render_heap(dag: &Graph, sgs: usize) {
    let start = Instant::now();
    let ir = dag.compute_layout();
    let output = ir.render_scanline();
    let elapsed = start.elapsed();

    let lines = output.lines().count();
    let max_width = output.lines().map(|l| l.len()).max().unwrap_or(0);

    println!("{}", output);
    println!(
        "  [Heap] {} subgraphs → {}×{} chars, {:?}",
        sgs, max_width, lines, elapsed
    );
}

#[cfg(feature = "arena")]
fn render_csr(dag: &Graph, sgs: usize) {
    let csr_size = dag.estimate_csr_arena_size() * 2;
    let mut csr_buf = vec![0u8; csr_size];
    let mut csr_arena = Arena::new(&mut csr_buf);
    let csr = match dag.to_csr(&mut csr_arena) {
        Some(g) => g,
        None => {
            println!(
                "  (CSR conversion failed — arena too small: {} KB)",
                csr_size / 1024
            );
            return;
        }
    };

    let layout_size = dag.estimate_layout_arena_size();
    let size = ((layout_size * 6) / 5).max(256 * 1024);
    let mut temp_buf = vec![0u8; size];
    let mut out_buf = vec![0u8; size];
    let mut temp_arena = Arena::new(&mut temp_buf);
    let mut out_arena = Arena::new(&mut out_buf);

    let start = Instant::now();

    let ir = match csr.compute_layout_arena(
        &LayoutConfig::standard(),
        &mut temp_arena,
        &mut out_arena,
    ) {
        Ok(ir) => ir,
        Err(e) => {
            println!("  (Layout failed: {:?}, arena: {} KB)", e, size / 1024);
            return;
        }
    };

    let (render_bytes, _) = ir.estimate_render_size();
    let rsize = render_bytes * 4 + 8192;
    let mut render_buf = vec![0u8; rsize];
    let mut line_buf = vec![' '; ir.width().max(1) + 32];
    let mut scratch = vec![0usize; (ir.height() + ir.edge_count() * 2).max(1) + 64];

    let bytes = ir
        .render_to_buffer(&mut render_buf, &mut line_buf, &mut scratch)
        .unwrap_or(0);
    let elapsed = start.elapsed();

    if let Ok(s) = std::str::from_utf8(&render_buf[..bytes]) {
        let lines = s.lines().count();
        let max_width = s.lines().map(|l| l.len()).max().unwrap_or(0);
        println!("{}", s);
        println!(
            "  [CSR]  {} subgraphs → {}×{} chars, {:?} (arena: {} KB)",
            sgs,
            max_width,
            lines,
            elapsed,
            (csr_size + size * 2) / 1024
        );
    }
}

// ── Tier 1: Microservices ────────────────────────────────────────────────
// 12 nodes, 4 subgraphs, depth 1 — the baseline "should look great"

fn tier1_microservices() -> Graph<'static> {
    let mut g = Graph::new();

    // Gateway layer
    g.add_node(1, "API-GW");

    // Auth service
    g.add_node(10, "AuthSvc");
    g.add_node(11, "TokenStore");

    // User service
    g.add_node(20, "UserSvc");
    g.add_node(21, "UserDB");

    // Order service
    g.add_node(30, "OrderSvc");
    g.add_node(31, "OrderDB");
    g.add_node(32, "OrderQueue");

    // Notification
    g.add_node(40, "NotifySvc");
    g.add_node(41, "EmailGW");
    g.add_node(42, "PushGW");

    // Edges: gateway fans out to services
    g.add_edge(1, 10, None);
    g.add_edge(1, 20, None);
    g.add_edge(1, 30, None);
    g.add_edge(10, 11, None);
    g.add_edge(20, 21, None);
    g.add_edge(30, 31, None);
    g.add_edge(30, 32, None);
    g.add_edge(32, 40, None);
    g.add_edge(40, 41, None);
    g.add_edge(40, 42, None);

    let auth = g.add_subgraph("Auth");
    let users = g.add_subgraph("Users");
    let orders = g.add_subgraph("Orders");
    let notify = g.add_subgraph("Notifications");
    g.put_nodes(&[10, 11]).inside(auth).unwrap();
    g.put_nodes(&[20, 21]).inside(users).unwrap();
    g.put_nodes(&[30, 31, 32]).inside(orders).unwrap();
    g.put_nodes(&[40, 41, 42]).inside(notify).unwrap();

    g
}

// ── Tier 2: Platform ─────────────────────────────────────────────────────
// 20 nodes, 8 subgraphs, depth 2 — nested services

fn tier2_platform() -> Graph<'static> {
    let mut g = Graph::new();

    g.add_node(1, "LoadBalancer");
    g.add_node(2, "CDN");

    // Frontend cluster
    g.add_node(10, "WebApp");
    g.add_node(11, "MobileAPI");
    g.add_node(12, "SSR-Engine");

    // Backend → Core
    g.add_node(20, "AuthSvc");
    g.add_node(21, "SessionDB");
    g.add_node(22, "UserSvc");
    g.add_node(23, "ProfileDB");

    // Backend → Data
    g.add_node(30, "Analytics");
    g.add_node(31, "Warehouse");
    g.add_node(32, "ETL");

    // Infra
    g.add_node(40, "Prometheus");
    g.add_node(41, "Grafana");
    g.add_node(42, "AlertMgr");
    g.add_node(43, "PagerDuty");

    // Logging
    g.add_node(50, "FluentBit");
    g.add_node(51, "ElasticSearch");
    g.add_node(52, "Kibana");

    // Edges
    g.add_edge(1, 10, None);
    g.add_edge(1, 11, None);
    g.add_edge(2, 12, None);
    g.add_edge(10, 20, None);
    g.add_edge(11, 20, None);
    g.add_edge(12, 22, None);
    g.add_edge(20, 21, None);
    g.add_edge(22, 23, None);
    g.add_edge(20, 30, None);
    g.add_edge(22, 30, None);
    g.add_edge(30, 31, None);
    g.add_edge(30, 32, None);
    g.add_edge(31, 40, None);
    g.add_edge(40, 41, None);
    g.add_edge(40, 42, None);
    g.add_edge(42, 43, None);
    g.add_edge(32, 50, None);
    g.add_edge(50, 51, None);
    g.add_edge(51, 52, None);

    // Subgraphs
    let frontend = g.add_subgraph("Frontend");
    let backend = g.add_subgraph("Backend");
    let core = g.add_subgraph("Core");
    let data = g.add_subgraph("Data Pipeline");
    let infra = g.add_subgraph("Infra");
    let monitoring = g.add_subgraph("Monitoring");
    let logging = g.add_subgraph("Logging");
    let observability = g.add_subgraph("Observability");

    g.put_nodes(&[10, 11, 12]).inside(frontend).unwrap();
    g.put_nodes(&[20, 21, 22, 23]).inside(core).unwrap();
    g.put_nodes(&[30, 31, 32]).inside(data).unwrap();
    g.put_subgraphs(&[core, data]).inside(backend).unwrap();
    g.put_nodes(&[40, 41, 42, 43]).inside(monitoring).unwrap();
    g.put_nodes(&[50, 51, 52]).inside(logging).unwrap();
    g.put_subgraphs(&[monitoring, logging])
        .inside(observability)
        .unwrap();
    // infra wraps observability
    g.put_subgraphs(&[observability]).inside(infra).unwrap();

    g
}

// ── Tier 3: Cloud Infra ──────────────────────────────────────────────────
// 30 nodes, 12 subgraphs, depth 3

fn tier3_cloud() -> Graph<'static> {
    let mut g = Graph::new();
    let mut id = 0usize;
    let mut next = || {
        id += 1;
        id
    };

    // Ingress
    let lb = next();
    g.add_node(lb, "ALB");
    let waf = next();
    g.add_node(waf, "WAF");
    g.add_edge(lb, waf, None);

    // Region A — AZ1
    let a1_web = next();
    g.add_node(a1_web, "Web-A1");
    let a1_app = next();
    g.add_node(a1_app, "App-A1");
    let a1_db = next();
    g.add_node(a1_db, "DB-A1");
    g.add_edge(waf, a1_web, None);
    g.add_edge(a1_web, a1_app, None);
    g.add_edge(a1_app, a1_db, None);

    // Region A — AZ2
    let a2_web = next();
    g.add_node(a2_web, "Web-A2");
    let a2_app = next();
    g.add_node(a2_app, "App-A2");
    let a2_db = next();
    g.add_node(a2_db, "DB-A2");
    g.add_edge(waf, a2_web, None);
    g.add_edge(a2_web, a2_app, None);
    g.add_edge(a2_app, a2_db, None);

    // Region B — AZ1
    let b1_web = next();
    g.add_node(b1_web, "Web-B1");
    let b1_app = next();
    g.add_node(b1_app, "App-B1");
    let b1_cache = next();
    g.add_node(b1_cache, "Redis-B1");
    let b1_db = next();
    g.add_node(b1_db, "DB-B1");
    g.add_edge(waf, b1_web, None);
    g.add_edge(b1_web, b1_app, None);
    g.add_edge(b1_app, b1_cache, None);
    g.add_edge(b1_cache, b1_db, None);

    // Region B — AZ2
    let b2_web = next();
    g.add_node(b2_web, "Web-B2");
    let b2_app = next();
    g.add_node(b2_app, "App-B2");
    let b2_cache = next();
    g.add_node(b2_cache, "Redis-B2");
    let b2_db = next();
    g.add_node(b2_db, "DB-B2");
    g.add_edge(waf, b2_web, None);
    g.add_edge(b2_web, b2_app, None);
    g.add_edge(b2_app, b2_cache, None);
    g.add_edge(b2_cache, b2_db, None);

    // Shared services
    let mq = next();
    g.add_node(mq, "RabbitMQ");
    let s3 = next();
    g.add_node(s3, "S3-Bucket");
    let vault = next();
    g.add_node(vault, "Vault");
    g.add_edge(a1_app, mq, None);
    g.add_edge(b1_app, mq, None);
    g.add_edge(mq, s3, None);

    // CI/CD
    let ci = next();
    g.add_node(ci, "Jenkins");
    let cd = next();
    g.add_node(cd, "ArgoCD");
    let registry = next();
    g.add_node(registry, "ECR");
    g.add_edge(ci, registry, None);
    g.add_edge(registry, cd, None);
    g.add_edge(cd, a1_web, None);
    g.add_edge(cd, b1_web, None);

    // Monitoring
    let prom = next();
    g.add_node(prom, "Prometheus");
    let grafana = next();
    g.add_node(grafana, "Grafana");
    let loki = next();
    g.add_node(loki, "Loki");
    g.add_edge(prom, grafana, None);
    g.add_edge(loki, grafana, None);
    g.add_edge(a1_app, prom, None);
    g.add_edge(b1_app, loki, None);

    // Subgraph hierarchy: Cloud → Region → AZ → nodes
    let az_a1 = g.add_subgraph("AZ-1");
    let az_a2 = g.add_subgraph("AZ-2");
    let region_a = g.add_subgraph("us-east-1");
    g.put_nodes(&[a1_web, a1_app, a1_db]).inside(az_a1).unwrap();
    g.put_nodes(&[a2_web, a2_app, a2_db]).inside(az_a2).unwrap();
    g.put_subgraphs(&[az_a1, az_a2]).inside(region_a).unwrap();

    let az_b1 = g.add_subgraph("AZ-1");
    let az_b2 = g.add_subgraph("AZ-2");
    let region_b = g.add_subgraph("eu-west-1");
    g.put_nodes(&[b1_web, b1_app, b1_cache, b1_db])
        .inside(az_b1)
        .unwrap();
    g.put_nodes(&[b2_web, b2_app, b2_cache, b2_db])
        .inside(az_b2)
        .unwrap();
    g.put_subgraphs(&[az_b1, az_b2]).inside(region_b).unwrap();

    let shared = g.add_subgraph("Shared Services");
    g.put_nodes(&[mq, s3, vault]).inside(shared).unwrap();

    let cicd = g.add_subgraph("CI/CD");
    g.put_nodes(&[ci, cd, registry]).inside(cicd).unwrap();

    let obs = g.add_subgraph("Observability");
    g.put_nodes(&[prom, grafana, loki]).inside(obs).unwrap();

    g
}

// ── Tier 4: Enterprise ───────────────────────────────────────────────────
// 50 nodes, 16 subgraphs, depth 3

fn tier4_enterprise() -> Graph<'static> {
    let mut g = Graph::new();
    let mut id = 0usize;
    let mut next = || {
        id += 1;
        id
    };

    // CEO-level entry points
    let portal = next();
    g.add_node(portal, "Portal");
    let mobile = next();
    g.add_node(mobile, "MobileApp");

    // Auth domain
    let auth_gw = next();
    g.add_node(auth_gw, "AuthGW");
    let oauth = next();
    g.add_node(oauth, "OAuth2");
    let ldap = next();
    g.add_node(ldap, "LDAP");
    let mfa = next();
    g.add_node(mfa, "MFA");

    // Customer domain
    let cust_api = next();
    g.add_node(cust_api, "CustAPI");
    let cust_svc = next();
    g.add_node(cust_svc, "CustSvc");
    let cust_db = next();
    g.add_node(cust_db, "CustDB");
    let cust_cache = next();
    g.add_node(cust_cache, "CustCache");

    // Product domain
    let prod_api = next();
    g.add_node(prod_api, "ProdAPI");
    let catalog = next();
    g.add_node(catalog, "Catalog");
    let inventory = next();
    g.add_node(inventory, "Inventory");
    let pricing = next();
    g.add_node(pricing, "Pricing");
    let prod_db = next();
    g.add_node(prod_db, "ProdDB");

    // Order domain
    let order_api = next();
    g.add_node(order_api, "OrderAPI");
    let order_svc = next();
    g.add_node(order_svc, "OrderSvc");
    let order_db = next();
    g.add_node(order_db, "OrderDB");
    let cart = next();
    g.add_node(cart, "CartSvc");
    let cart_db = next();
    g.add_node(cart_db, "CartDB");

    // Payment domain
    let pay_gw = next();
    g.add_node(pay_gw, "PayGW");
    let pay_proc = next();
    g.add_node(pay_proc, "PayProc");
    let pay_db = next();
    g.add_node(pay_db, "PayDB");
    let fraud = next();
    g.add_node(fraud, "FraudDet");

    // Shipping domain
    let ship_api = next();
    g.add_node(ship_api, "ShipAPI");
    let ship_svc = next();
    g.add_node(ship_svc, "ShipSvc");
    let ship_db = next();
    g.add_node(ship_db, "ShipDB");
    let tracking = next();
    g.add_node(tracking, "Tracking");

    // Messaging backbone
    let kafka = next();
    g.add_node(kafka, "Kafka");
    let schema_reg = next();
    g.add_node(schema_reg, "SchemaReg");

    // Data Platform
    let spark = next();
    g.add_node(spark, "Spark");
    let datalake = next();
    g.add_node(datalake, "DataLake");
    let airflow = next();
    g.add_node(airflow, "Airflow");
    let redshift = next();
    g.add_node(redshift, "Redshift");

    // Observability
    let prom = next();
    g.add_node(prom, "Prometheus");
    let grafana = next();
    g.add_node(grafana, "Grafana");
    let jaeger = next();
    g.add_node(jaeger, "Jaeger");
    let elk = next();
    g.add_node(elk, "ELK");
    let pager = next();
    g.add_node(pager, "PagerDuty");

    // DevOps
    let github = next();
    g.add_node(github, "GitHub");
    let ci = next();
    g.add_node(ci, "CI-Runner");
    let terraform = next();
    g.add_node(terraform, "Terraform");
    let k8s = next();
    g.add_node(k8s, "K8s");
    let helm = next();
    g.add_node(helm, "Helm");

    // Notification
    let notif = next();
    g.add_node(notif, "NotifSvc");
    let email = next();
    g.add_node(email, "EmailGW");
    let sms = next();
    g.add_node(sms, "SMS-GW");
    let push = next();
    g.add_node(push, "PushSvc");

    // Edges — entry points
    g.add_edge(portal, auth_gw, None);
    g.add_edge(mobile, auth_gw, None);
    g.add_edge(auth_gw, oauth, None);
    g.add_edge(auth_gw, ldap, None);
    g.add_edge(oauth, mfa, None);

    // Auth → domains
    g.add_edge(auth_gw, cust_api, None);
    g.add_edge(auth_gw, prod_api, None);
    g.add_edge(auth_gw, order_api, None);

    // Customer
    g.add_edge(cust_api, cust_svc, None);
    g.add_edge(cust_svc, cust_db, None);
    g.add_edge(cust_svc, cust_cache, None);

    // Product
    g.add_edge(prod_api, catalog, None);
    g.add_edge(prod_api, inventory, None);
    g.add_edge(catalog, pricing, None);
    g.add_edge(catalog, prod_db, None);
    g.add_edge(inventory, prod_db, None);

    // Order
    g.add_edge(order_api, order_svc, None);
    g.add_edge(order_api, cart, None);
    g.add_edge(order_svc, order_db, None);
    g.add_edge(cart, cart_db, None);
    g.add_edge(order_svc, pay_gw, None);
    g.add_edge(order_svc, ship_api, None);

    // Payment
    g.add_edge(pay_gw, pay_proc, None);
    g.add_edge(pay_proc, pay_db, None);
    g.add_edge(pay_proc, fraud, None);

    // Shipping
    g.add_edge(ship_api, ship_svc, None);
    g.add_edge(ship_svc, ship_db, None);
    g.add_edge(ship_svc, tracking, None);

    // Messaging
    g.add_edge(order_svc, kafka, None);
    g.add_edge(pay_proc, kafka, None);
    g.add_edge(ship_svc, kafka, None);
    g.add_edge(kafka, schema_reg, None);

    // Data platform
    g.add_edge(kafka, spark, None);
    g.add_edge(spark, datalake, None);
    g.add_edge(airflow, spark, None);
    g.add_edge(datalake, redshift, None);

    // Observability
    g.add_edge(kafka, prom, None);
    g.add_edge(prom, grafana, None);
    g.add_edge(prom, jaeger, None);
    g.add_edge(prom, elk, None);
    g.add_edge(grafana, pager, None);

    // Notifications
    g.add_edge(order_svc, notif, None);
    g.add_edge(notif, email, None);
    g.add_edge(notif, sms, None);
    g.add_edge(notif, push, None);

    // DevOps
    g.add_edge(github, ci, None);
    g.add_edge(ci, helm, None);
    g.add_edge(helm, k8s, None);
    g.add_edge(terraform, k8s, None);

    // Subgraphs
    let sg_auth = g.add_subgraph("Identity");
    g.put_nodes(&[auth_gw, oauth, ldap, mfa])
        .inside(sg_auth)
        .unwrap();

    let sg_cust = g.add_subgraph("Customer");
    g.put_nodes(&[cust_api, cust_svc, cust_db, cust_cache])
        .inside(sg_cust)
        .unwrap();

    let sg_catalog = g.add_subgraph("Catalog");
    g.put_nodes(&[catalog, pricing, prod_db])
        .inside(sg_catalog)
        .unwrap();
    let sg_prod = g.add_subgraph("Product");
    g.put_nodes(&[prod_api, inventory]).inside(sg_prod).unwrap();
    g.put_subgraphs(&[sg_catalog]).inside(sg_prod).unwrap();

    let sg_cart = g.add_subgraph("Cart");
    g.put_nodes(&[cart, cart_db]).inside(sg_cart).unwrap();
    let sg_order = g.add_subgraph("Orders");
    g.put_nodes(&[order_api, order_svc, order_db])
        .inside(sg_order)
        .unwrap();
    g.put_subgraphs(&[sg_cart]).inside(sg_order).unwrap();

    let sg_pay = g.add_subgraph("Payments");
    g.put_nodes(&[pay_gw, pay_proc, pay_db, fraud])
        .inside(sg_pay)
        .unwrap();

    let sg_ship = g.add_subgraph("Shipping");
    g.put_nodes(&[ship_api, ship_svc, ship_db, tracking])
        .inside(sg_ship)
        .unwrap();

    let sg_msg = g.add_subgraph("Messaging");
    g.put_nodes(&[kafka, schema_reg]).inside(sg_msg).unwrap();

    let sg_dp = g.add_subgraph("Data Platform");
    g.put_nodes(&[spark, datalake, airflow, redshift])
        .inside(sg_dp)
        .unwrap();

    let sg_mon = g.add_subgraph("Monitoring");
    g.put_nodes(&[prom, grafana, jaeger, elk, pager])
        .inside(sg_mon)
        .unwrap();

    let sg_obs = g.add_subgraph("Observability");
    g.put_subgraphs(&[sg_mon]).inside(sg_obs).unwrap();

    let sg_notif = g.add_subgraph("Notifications");
    g.put_nodes(&[notif, email, sms, push])
        .inside(sg_notif)
        .unwrap();

    let sg_devops = g.add_subgraph("DevOps");
    g.put_nodes(&[github, ci, terraform, k8s, helm])
        .inside(sg_devops)
        .unwrap();

    g
}

// ── Tier 5: Megacorp ─────────────────────────────────────────────────────
// 80 nodes, 24 subgraphs, depth 4  — the "will my terminal survive?" test

fn tier5_megacorp() -> Graph<'static> {
    let mut g = Graph::new();
    let mut id = 0usize;
    let mut next = || {
        id += 1;
        id
    };

    // ── Global Entry ──
    let dns = next();
    g.add_node(dns, "Route53");
    let cdn = next();
    g.add_node(cdn, "CloudFront");
    let waf = next();
    g.add_node(waf, "WAF");
    g.add_edge(dns, cdn, None);
    g.add_edge(cdn, waf, None);

    // ── US Region ──
    //   Frontend
    let us_web = next();
    g.add_node(us_web, "US-Web");
    let us_mobile = next();
    g.add_node(us_mobile, "US-Mobile");
    let us_ssr = next();
    g.add_node(us_ssr, "US-SSR");
    g.add_edge(waf, us_web, None);
    g.add_edge(waf, us_mobile, None);
    g.add_edge(us_web, us_ssr, None);

    //   Auth micro
    let us_auth = next();
    g.add_node(us_auth, "US-Auth");
    let us_token = next();
    g.add_node(us_token, "US-Token");
    let us_mfa = next();
    g.add_node(us_mfa, "US-MFA");
    g.add_edge(us_web, us_auth, None);
    g.add_edge(us_mobile, us_auth, None);
    g.add_edge(us_auth, us_token, None);
    g.add_edge(us_auth, us_mfa, None);

    //   Biz logic
    let us_order = next();
    g.add_node(us_order, "US-Orders");
    let us_pay = next();
    g.add_node(us_pay, "US-Pay");
    let us_inv = next();
    g.add_node(us_inv, "US-Inv");
    let us_ship = next();
    g.add_node(us_ship, "US-Ship");
    g.add_edge(us_auth, us_order, None);
    g.add_edge(us_order, us_pay, None);
    g.add_edge(us_order, us_inv, None);
    g.add_edge(us_order, us_ship, None);

    //   Databases
    let us_pg = next();
    g.add_node(us_pg, "US-Postgres");
    let us_redis = next();
    g.add_node(us_redis, "US-Redis");
    let us_s3 = next();
    g.add_node(us_s3, "US-S3");
    g.add_edge(us_order, us_pg, None);
    g.add_edge(us_pay, us_pg, None);
    g.add_edge(us_auth, us_redis, None);
    g.add_edge(us_inv, us_s3, None);

    // ── EU Region (mirror, smaller) ──
    let eu_web = next();
    g.add_node(eu_web, "EU-Web");
    let eu_auth = next();
    g.add_node(eu_auth, "EU-Auth");
    let eu_order = next();
    g.add_node(eu_order, "EU-Orders");
    let eu_pay = next();
    g.add_node(eu_pay, "EU-Pay");
    let eu_ship = next();
    g.add_node(eu_ship, "EU-Ship");
    let eu_pg = next();
    g.add_node(eu_pg, "EU-Postgres");
    let eu_redis = next();
    g.add_node(eu_redis, "EU-Redis");
    g.add_edge(waf, eu_web, None);
    g.add_edge(eu_web, eu_auth, None);
    g.add_edge(eu_auth, eu_order, None);
    g.add_edge(eu_order, eu_pay, None);
    g.add_edge(eu_order, eu_ship, None);
    g.add_edge(eu_order, eu_pg, None);
    g.add_edge(eu_auth, eu_redis, None);

    // ── APAC Region ──
    let ap_web = next();
    g.add_node(ap_web, "AP-Web");
    let ap_auth = next();
    g.add_node(ap_auth, "AP-Auth");
    let ap_order = next();
    g.add_node(ap_order, "AP-Orders");
    let ap_pg = next();
    g.add_node(ap_pg, "AP-Postgres");
    g.add_edge(waf, ap_web, None);
    g.add_edge(ap_web, ap_auth, None);
    g.add_edge(ap_auth, ap_order, None);
    g.add_edge(ap_order, ap_pg, None);

    // ── Messaging Layer ──
    let kafka1 = next();
    g.add_node(kafka1, "Kafka-1");
    let kafka2 = next();
    g.add_node(kafka2, "Kafka-2");
    let zk = next();
    g.add_node(zk, "Zookeeper");
    let schema = next();
    g.add_node(schema, "SchemaReg");
    g.add_edge(us_order, kafka1, None);
    g.add_edge(eu_order, kafka1, None);
    g.add_edge(kafka1, kafka2, None);
    g.add_edge(kafka1, zk, None);
    g.add_edge(kafka2, schema, None);

    // ── Data Platform ──
    let spark = next();
    g.add_node(spark, "Spark");
    let flink = next();
    g.add_node(flink, "Flink");
    let airflow = next();
    g.add_node(airflow, "Airflow");
    let datalake = next();
    g.add_node(datalake, "DataLake");
    let redshift = next();
    g.add_node(redshift, "Redshift");
    let tableau = next();
    g.add_node(tableau, "Tableau");
    g.add_edge(kafka2, spark, None);
    g.add_edge(kafka2, flink, None);
    g.add_edge(airflow, spark, None);
    g.add_edge(spark, datalake, None);
    g.add_edge(flink, datalake, None);
    g.add_edge(datalake, redshift, None);
    g.add_edge(redshift, tableau, None);

    // ── ML Platform ──
    let mlflow = next();
    g.add_node(mlflow, "MLflow");
    let sagemaker = next();
    g.add_node(sagemaker, "SageMaker");
    let model_reg = next();
    g.add_node(model_reg, "ModelReg");
    let inference = next();
    g.add_node(inference, "Inference");
    g.add_edge(datalake, mlflow, None);
    g.add_edge(mlflow, sagemaker, None);
    g.add_edge(sagemaker, model_reg, None);
    g.add_edge(model_reg, inference, None);
    g.add_edge(inference, us_order, None); // predictions feed back

    // ── Observability ──
    let prom = next();
    g.add_node(prom, "Prometheus");
    let grafana = next();
    g.add_node(grafana, "Grafana");
    let jaeger = next();
    g.add_node(jaeger, "Jaeger");
    let loki = next();
    g.add_node(loki, "Loki");
    let cortex = next();
    g.add_node(cortex, "Cortex");
    let pager = next();
    g.add_node(pager, "PagerDuty");
    let opsgenie = next();
    g.add_node(opsgenie, "OpsGenie");
    g.add_edge(us_order, prom, None);
    g.add_edge(eu_order, prom, None);
    g.add_edge(prom, cortex, None);
    g.add_edge(cortex, grafana, None);
    g.add_edge(prom, jaeger, None);
    g.add_edge(prom, loki, None);
    g.add_edge(grafana, pager, None);
    g.add_edge(grafana, opsgenie, None);

    // ── Security ──
    let vault = next();
    g.add_node(vault, "Vault");
    let cert_mgr = next();
    g.add_node(cert_mgr, "CertMgr");
    let guard = next();
    g.add_node(guard, "GuardDuty");
    let inspector = next();
    g.add_node(inspector, "Inspector");
    g.add_edge(us_auth, vault, None);
    g.add_edge(eu_auth, vault, None);
    g.add_edge(vault, cert_mgr, None);
    g.add_edge(vault, guard, None);
    g.add_edge(guard, inspector, None);

    // ── DevOps / Platform ──
    let github = next();
    g.add_node(github, "GitHub");
    let ci = next();
    g.add_node(ci, "Actions");
    let ecr = next();
    g.add_node(ecr, "ECR");
    let argo = next();
    g.add_node(argo, "ArgoCD");
    let tf = next();
    g.add_node(tf, "Terraform");
    let k8s_us = next();
    g.add_node(k8s_us, "EKS-US");
    let k8s_eu = next();
    g.add_node(k8s_eu, "EKS-EU");
    let k8s_ap = next();
    g.add_node(k8s_ap, "EKS-AP");
    g.add_edge(github, ci, None);
    g.add_edge(ci, ecr, None);
    g.add_edge(ecr, argo, None);
    g.add_edge(argo, k8s_us, None);
    g.add_edge(argo, k8s_eu, None);
    g.add_edge(argo, k8s_ap, None);
    g.add_edge(tf, k8s_us, None);
    g.add_edge(tf, k8s_eu, None);
    g.add_edge(tf, k8s_ap, None);

    // ── Notifications ──
    let sns = next();
    g.add_node(sns, "SNS");
    let ses = next();
    g.add_node(ses, "SES");
    let slack_hook = next();
    g.add_node(slack_hook, "Slack");
    g.add_edge(pager, sns, None);
    g.add_edge(sns, ses, None);
    g.add_edge(sns, slack_hook, None);

    // ── Build subgraph hierarchy ──

    // US Region → Frontend, Auth, Business, Data
    let us_fe = g.add_subgraph("US-Frontend");
    g.put_nodes(&[us_web, us_mobile, us_ssr])
        .inside(us_fe)
        .unwrap();
    let us_au = g.add_subgraph("US-Auth");
    g.put_nodes(&[us_auth, us_token, us_mfa])
        .inside(us_au)
        .unwrap();
    let us_biz = g.add_subgraph("US-Business");
    g.put_nodes(&[us_order, us_pay, us_inv, us_ship])
        .inside(us_biz)
        .unwrap();
    let us_data = g.add_subgraph("US-Data");
    g.put_nodes(&[us_pg, us_redis, us_s3])
        .inside(us_data)
        .unwrap();
    let region_us = g.add_subgraph("US-East");
    g.put_subgraphs(&[us_fe, us_au, us_biz, us_data])
        .inside(region_us)
        .unwrap();

    // EU Region
    let eu_svc = g.add_subgraph("EU-Services");
    g.put_nodes(&[eu_web, eu_auth, eu_order, eu_pay, eu_ship])
        .inside(eu_svc)
        .unwrap();
    let eu_db = g.add_subgraph("EU-Data");
    g.put_nodes(&[eu_pg, eu_redis]).inside(eu_db).unwrap();
    let region_eu = g.add_subgraph("EU-West");
    g.put_subgraphs(&[eu_svc, eu_db]).inside(region_eu).unwrap();

    // APAC Region
    let region_ap = g.add_subgraph("APAC");
    g.put_nodes(&[ap_web, ap_auth, ap_order, ap_pg])
        .inside(region_ap)
        .unwrap();

    // Messaging
    let sg_msg = g.add_subgraph("Event Bus");
    g.put_nodes(&[kafka1, kafka2, zk, schema])
        .inside(sg_msg)
        .unwrap();

    // Data Platform
    let sg_ingest = g.add_subgraph("Ingestion");
    g.put_nodes(&[spark, flink]).inside(sg_ingest).unwrap();
    let sg_store = g.add_subgraph("Storage");
    g.put_nodes(&[datalake, redshift]).inside(sg_store).unwrap();
    let sg_dp = g.add_subgraph("Data Platform");
    g.put_nodes(&[airflow, tableau]).inside(sg_dp).unwrap();
    g.put_subgraphs(&[sg_ingest, sg_store])
        .inside(sg_dp)
        .unwrap();

    // ML Platform
    let sg_ml = g.add_subgraph("ML Platform");
    g.put_nodes(&[mlflow, sagemaker, model_reg, inference])
        .inside(sg_ml)
        .unwrap();

    // Observability
    let sg_metrics = g.add_subgraph("Metrics");
    g.put_nodes(&[prom, cortex, grafana])
        .inside(sg_metrics)
        .unwrap();
    let sg_tracing = g.add_subgraph("Tracing");
    g.put_nodes(&[jaeger, loki]).inside(sg_tracing).unwrap();
    let sg_alert = g.add_subgraph("Alerting");
    g.put_nodes(&[pager, opsgenie]).inside(sg_alert).unwrap();
    let sg_obs = g.add_subgraph("Observability");
    g.put_subgraphs(&[sg_metrics, sg_tracing, sg_alert])
        .inside(sg_obs)
        .unwrap();

    // Security
    let sg_sec = g.add_subgraph("Security");
    g.put_nodes(&[vault, cert_mgr, guard, inspector])
        .inside(sg_sec)
        .unwrap();

    // DevOps
    let sg_cicd = g.add_subgraph("CI/CD");
    g.put_nodes(&[github, ci, ecr, argo])
        .inside(sg_cicd)
        .unwrap();
    let sg_infra = g.add_subgraph("Infrastructure");
    g.put_nodes(&[tf, k8s_us, k8s_eu, k8s_ap])
        .inside(sg_infra)
        .unwrap();
    let sg_devops = g.add_subgraph("Platform Eng");
    g.put_subgraphs(&[sg_cicd, sg_infra])
        .inside(sg_devops)
        .unwrap();

    // Notifications
    let sg_notif = g.add_subgraph("Notifications");
    g.put_nodes(&[sns, ses, slack_hook])
        .inside(sg_notif)
        .unwrap();

    g
}
