//! Fake reply helper — builds a `ContextInfo` with a synthesized `QuotedMessage`
//! so the outgoing message appears to be a reply to a previous (non-existent)
//! message. Used by blast to appear more natural / human-like.

use crate::models::messages::FakeReplyConfig;
use rand::seq::SliceRandom;
use rand::Rng;
use waproto::whatsapp as wa;

// ── Indonesian realistic data pools ─────────────────────────────────────────

const PRODUCT_NAMES: &[&str] = &[
    "Sepatu Nike Air Max 270",
    "Kaos Polos Cotton Combed 30s",
    "Celana Jeans Slim Fit Pria",
    "Tas Ransel Laptop Anti Air",
    "Jam Tangan Casio G-Shock",
    "Kemeja Flannel Kotak-Kotak",
    "Sneakers Adidas Ultraboost",
    "Hoodie Oversize Unisex",
    "Dompet Kulit Asli Pria",
    "Kacamata Hitam Polarized UV400",
    "Topi Baseball NY Cap Original",
    "Parfum EDT 100ml Pria",
    "Jaket Bomber Premium",
    "Dress Wanita Casual Korean",
    "Skincare Paket Lengkap BPOM",
    "Serum Vitamin C 20%",
    "Lipstik Matte Waterproof",
    "Sunscreen SPF 50+ PA++++",
    "iPhone 15 Pro Max 256GB",
    "Samsung Galaxy S24 Ultra",
    "Airpods Pro 2nd Gen",
    "Mouse Gaming Logitech G502",
    "Headset Bluetooth ANC",
    "Charger Fast Charging 65W",
    "Powerbank 20000mAh PD",
    "Minyak Goreng 2 Liter",
    "Beras Premium 5kg",
    "Kopi Arabica Gayo 250gr",
    "Madu Asli Hutan 500ml",
];

const PRODUCT_DESCRIPTIONS: &[&str] = &[
    "Ready stock, bisa COD",
    "Termurah se-marketplace!",
    "Best seller bulan ini",
    "Kualitas premium, harga terjangkau",
    "Garansi 1 tahun resmi",
    "Free ongkir seluruh Indonesia",
    "Original 100% money back guarantee",
    "Promo terbatas hari ini",
    "Stok terbatas, order sekarang",
    "Rating 4.9 / 5.0",
    "Sudah terjual 10rb+",
    "Flash sale tinggal 50 pcs",
];

const STORE_NAMES: &[&str] = &[
    "TokoMaju_Official",
    "BerkahShop88",
    "SinarJaya_Store",
    "MegaMart_ID",
    "PrimaOlshop",
    "GadgetPlaza_ID",
    "FashionHub.co",
    "BeautyQueen_Official",
    "SportZone_ID",
    "TechWorld_Store",
];

const LOCATIONS: &[(&str, f64, f64)] = &[
    ("Grand Indonesia Shopping Town", -6.1952, 106.8216),
    ("Mal Kelapa Gading", -6.1579, 106.9082),
    ("Pondok Indah Mall", -6.2646, 106.7841),
    ("Trans Studio Mall Bandung", -6.9263, 107.6345),
    ("Pakuwon Mall Surabaya", -7.2902, 112.6763),
    ("Paragon City Mall Semarang", -6.9914, 110.4205),
    ("Hartono Mall Yogyakarta", -7.7466, 110.3977),
    ("Bali Galeria Mall Kuta", -8.7234, 115.1869),
    ("Summarecon Mall Bekasi", -6.2257, 107.0001),
    ("Aeon Mall BSD City", -6.3047, 106.6441),
    ("Tunjungan Plaza Surabaya", -7.2618, 112.7379),
    ("Sun Plaza Medan", 3.5737, 98.6875),
];

const CONTACT_NAMES: &[&str] = &[
    "Budi Santoso",
    "Dewi Lestari",
    "Ahmad Rizky",
    "Siti Nurhaliza",
    "Andi Pratama",
    "Rina Wati",
    "Dimas Arya",
    "Putri Ayu",
    "Hendra Wijaya",
    "Maya Sari",
];

const CONTACT_PHONES: &[&str] = &[
    "628131234567",
    "628521234567",
    "628561234567",
    "628131111222",
    "628529998877",
    "628136665544",
    "628527773322",
    "628561112233",
];

const DOCUMENT_NAMES: &[&str] = &[
    "Invoice_2024_00123.pdf",
    "Laporan_Keuangan_Q4.pdf",
    "Proposal_Bisnis_2024.pdf",
    "Katalog_Produk_Terbaru.pdf",
    "Daftar_Harga_Grosir.pdf",
    "Bukti_Transfer_BCA.pdf",
    "Sertifikat_BPOM.pdf",
    "Data_Pelanggan_2024.xlsx",
    "Rekap_Penjualan.xlsx",
    "E-Ticket_Pesawat.pdf",
];

const VIDEO_CAPTIONS: &[&str] = &[
    "Review produk lengkap",
    "Tutorial cara pakai",
    "Unboxing barang baru datang",
    "Testimoni customer puas",
    "Live demo produk asli",
    "Before & After pemakaian",
    "Bukti resi pengiriman hari ini",
];

const TEXT_MESSAGES: &[&str] = &[
    "Kak, barangnya ready ga?",
    "Min, bisa nego ga harganya?",
    "Kapan restock lagi?",
    "Bisa kirim hari ini?",
    "Ongkirnya berapa ke Jakarta?",
    "Ada diskon buat beli banyak?",
    "Bisa COD ga min?",
    "Mau order 5 pcs bisa?",
    "Sudah pernah pakai, recommended!",
    "Terima kasih kak barang sudah sampai",
    "Mantap kak, barangnya sesuai foto",
];

const IDR_PRICES_1000: &[i64] = &[
    15_000_000, 25_000_000, 35_000_000, 49_000_000, 75_000_000, 99_000_000, 150_000_000,
    199_000_000, 250_000_000, 350_000_000, 500_000_000, 750_000_000, 999_000_000,
];

fn pick<T: Copy>(arr: &[T]) -> T {
    let mut rng = rand::thread_rng();
    *arr.choose(&mut rng).expect("non-empty pool")
}

fn rand_digits(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len).map(|_| rng.gen_range(0..10).to_string()).collect()
}

fn random_jid() -> String {
    let prefixes = ["6281", "6282", "6285", "6287", "6288", "6289"];
    let mut rng = rand::thread_rng();
    let prefix = prefixes[rng.gen_range(0..prefixes.len())];
    let rest_len = 8 + rng.gen_range(0..=2);
    format!("{}{}@s.whatsapp.net", prefix, rand_digits(rest_len))
}

fn random_stanza_id() -> String {
    let mut rng = rand::thread_rng();
    let suffix: String = (0..16)
        .map(|_| {
            let c = b"ABCDEF0123456789"[rng.gen_range(0..16)] as char;
            c
        })
        .collect();
    format!("FR{}", suffix)
}

/// Build a ContextInfo that turns the outgoing message into a "reply" to a
/// synthesized quoted message. Returns `None` if type is unknown.
pub fn build_fake_reply_context_info(cfg: &FakeReplyConfig) -> Option<wa::ContextInfo> {
    let stanza_id = cfg
        .stanza_id
        .clone()
        .unwrap_or_else(random_stanza_id);
    let participant = cfg.participant.clone().unwrap_or_else(random_jid);

    let quoted = build_quoted_message(&cfg.reply_type, cfg.title.as_deref(), cfg.body.as_deref())?;

    Some(wa::ContextInfo {
        stanza_id: Some(stanza_id),
        participant: Some(participant),
        quoted_message: Some(Box::new(quoted)),
        ..Default::default()
    })
}

fn build_quoted_message(
    reply_type: &str,
    title: Option<&str>,
    body: Option<&str>,
) -> Option<wa::Message> {
    let mut rng = rand::thread_rng();

    match reply_type.to_lowercase().as_str() {
        "text" | "conversation" => {
            let text = body
                .map(|s| s.to_string())
                .or_else(|| title.map(|s| s.to_string()))
                .unwrap_or_else(|| pick(TEXT_MESSAGES).to_string());
            Some(wa::Message {
                conversation: Some(text),
                ..Default::default()
            })
        }

        "product" => {
            let product_title = title
                .map(|s| s.to_string())
                .unwrap_or_else(|| pick(PRODUCT_NAMES).to_string());
            let desc = body
                .map(|s| s.to_string())
                .unwrap_or_else(|| pick(PRODUCT_DESCRIPTIONS).to_string());
            let retailer = pick(STORE_NAMES).to_string();
            let price = pick(IDR_PRICES_1000);
            let business_jid = random_jid();

            Some(wa::Message {
                product_message: Some(Box::new(wa::message::ProductMessage {
                    product: Some(Box::new(wa::message::product_message::ProductSnapshot {
                        product_image: None,
                        product_id: Some(format!("PROD_{}", rand_digits(8))),
                        title: Some(product_title),
                        description: Some(desc),
                        currency_code: Some("IDR".to_string()),
                        price_amount1000: Some(price),
                        retailer_id: Some(retailer),
                        product_image_count: Some(rng.gen_range(1..=5)),
                        ..Default::default()
                    })),
                    business_owner_jid: Some(business_jid),
                    ..Default::default()
                })),
                ..Default::default()
            })
        }

        "order" => {
            let order_title = title
                .map(|s| s.to_string())
                .unwrap_or_else(|| pick(PRODUCT_NAMES).to_string());
            let msg = body.map(|s| s.to_string()).unwrap_or_else(|| {
                format!(
                    "Pesanan {} x{}",
                    pick(PRODUCT_NAMES),
                    rng.gen_range(1..=5)
                )
            });

            Some(wa::Message {
                order_message: Some(Box::new(wa::message::OrderMessage {
                    order_id: Some(format!("ORD_{}", rand_digits(10))),
                    item_count: Some(rng.gen_range(1..=8)),
                    status: Some(1),
                    surface: Some(1),
                    message: Some(msg),
                    order_title: Some(order_title),
                    seller_jid: Some(random_jid()),
                    ..Default::default()
                })),
                ..Default::default()
            })
        }

        "location" => {
            let (loc_name, lat, lng) = pick(LOCATIONS);
            let name_override = title.map(|s| s.to_string()).unwrap_or_else(|| loc_name.to_string());
            // Small jitter so it doesn't look too uniform
            let lat_jitter = rng.gen_range(-0.005..0.005);
            let lng_jitter = rng.gen_range(-0.005..0.005);
            Some(wa::Message {
                location_message: Some(Box::new(wa::message::LocationMessage {
                    degrees_latitude: Some(lat + lat_jitter),
                    degrees_longitude: Some(lng + lng_jitter),
                    name: Some(name_override),
                    address: body.map(|s| s.to_string()),
                    ..Default::default()
                })),
                ..Default::default()
            })
        }

        "video" => {
            let caption = body
                .map(|s| s.to_string())
                .or_else(|| title.map(|s| s.to_string()))
                .unwrap_or_else(|| pick(VIDEO_CAPTIONS).to_string());
            Some(wa::Message {
                video_message: Some(Box::new(wa::message::VideoMessage {
                    seconds: Some(rng.gen_range(5..=180)),
                    caption: Some(caption),
                    mimetype: Some("video/mp4".to_string()),
                    ..Default::default()
                })),
                ..Default::default()
            })
        }

        "document" => {
            let filename = title
                .map(|s| s.to_string())
                .unwrap_or_else(|| pick(DOCUMENT_NAMES).to_string());
            Some(wa::Message {
                document_message: Some(Box::new(wa::message::DocumentMessage {
                    title: Some(filename.clone()),
                    file_name: Some(filename),
                    mimetype: Some("application/pdf".to_string()),
                    caption: body.map(|s| s.to_string()),
                    ..Default::default()
                })),
                ..Default::default()
            })
        }

        "contact" => {
            let name = title.map(|s| s.to_string()).unwrap_or_else(|| pick(CONTACT_NAMES).to_string());
            let phone = pick(CONTACT_PHONES).to_string();
            let vcard = format!(
                "BEGIN:VCARD\nVERSION:3.0\nFN:{}\nTEL;type=CELL:+{}\nEND:VCARD",
                name, phone
            );
            Some(wa::Message {
                contact_message: Some(Box::new(wa::message::ContactMessage {
                    display_name: Some(name),
                    vcard: Some(vcard),
                    ..Default::default()
                })),
                ..Default::default()
            })
        }

        _ => None,
    }
}
