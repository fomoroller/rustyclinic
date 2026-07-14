//! Askama template structs for all pages and partials.

use askama::Template;

// ===== Auth pages =====

#[derive(Template)]
#[template(path = "pages/login.html")]
pub struct LoginPage {
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/lock_screen.html")]
pub struct LockScreenPage {
    pub session_id: String,
    pub display_name: String,
    pub initials: String,
    pub error: Option<String>,
}

// ===== App pages =====

#[derive(Template)]
#[template(path = "pages/patients/register.html")]
pub struct PatientRegisterPage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "pages/patients/search.html")]
pub struct PatientSearchPage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub patients: Vec<PatientView>,
}

#[derive(Template)]
#[template(path = "pages/patients/detail.html")]
pub struct PatientDetailPage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub patient_id: String,
    pub patient_name: String,
    pub patient_initials: String,
    pub sex: String,
    pub age: String,
    pub national_id: String,
    pub last_visit: String,
    pub active_programs: Vec<String>,
    pub timeline: Vec<PatientTimelineItemView>,
}

#[derive(Template)]
#[template(path = "pages/patients/patient_card_print.html")]
pub struct PatientCardPrintPage {
    pub patient_id: String,
    pub patient_name: String,
    pub national_id: String,
    pub sex: String,
    pub age: String,
    pub last_visit: String,
    pub active_programs: Vec<String>,
}

#[derive(Template)]
#[template(path = "pages/queue/board.html")]
pub struct QueueBoardPage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub entries: Vec<QueueEntryView>,
    pub waiting_count: u32,
    pub in_service_count: u32,
    pub completed_count: u32,
    pub avg_wait: String,
    pub department_filter: String,
}

#[derive(Template)]
#[template(path = "pages/queue/queue_ticket_print.html")]
pub struct QueueTicketPrintPage {
    pub queue_entry_id: String,
    pub position: u32,
    pub patient_name: String,
    pub department: String,
    pub service_type: String,
    pub status: String,
}

#[derive(Template)]
#[template(path = "pages/encounters/capture.html")]
pub struct EncounterCapturePage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub encounter_id: String,
    pub queue_entry_id: String,
    pub patient_name: String,
    pub patient_initials: String,
    pub service_type: String,
    pub form_html: String,
    pub form_error: Option<String>,
}

// ===== Simple queue board =====

#[derive(Template)]
#[template(path = "pages/queue/simple_board.html")]
pub struct SimpleQueueBoardPage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub patients: Vec<SimpleQueuePatientView>,
    pub waiting_count: u32,
    pub in_service_count: u32,
    pub completed_count: u32,
    pub avg_wait: String,
}

// ===== Triage page =====

#[derive(Template)]
#[template(path = "pages/encounters/triage.html")]
pub struct TriagePage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub encounter_id: String,
    pub queue_entry_id: String,
    pub patient_name: String,
    pub patient_initials: String,
    pub form_html: String,
    pub form_error: Option<String>,
}

// ===== Partials =====

#[derive(Template)]
#[template(path = "partials/patient_list.html")]
pub struct PatientListPartial {
    pub patients: Vec<PatientView>,
}

#[derive(Template)]
#[template(path = "partials/queue_entry_row.html")]
pub struct QueueEntryRowPartial {
    pub e: QueueEntryView,
}

/// Combined stats + entries partial for htmx polling.
#[derive(Template)]
#[template(path = "partials/queue_stats.html")]
pub struct QueueStatsPartial {
    pub waiting_count: u32,
    pub in_service_count: u32,
    pub completed_count: u32,
    pub avg_wait: String,
}

// ===== Order creation pages =====

#[derive(Template)]
#[template(path = "pages/encounters/order_lab.html")]
pub struct OrderLabPage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub encounter_id: String,
    pub queue_entry_id: String,
    pub patient_id: String,
    pub patient_name: String,
}

#[derive(Template)]
#[template(path = "pages/encounters/prescribe.html")]
pub struct PrescribePage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub encounter_id: String,
    pub queue_entry_id: String,
    pub patient_id: String,
    pub patient_name: String,
}

// ===== Lab & Pharmacy pages =====

#[derive(Template)]
#[template(path = "pages/lab/queue.html")]
pub struct LabQueuePage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub orders: Vec<LabOrderView>,
}

#[derive(Template)]
#[template(path = "pages/lab/results.html")]
pub struct LabResultsPage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub order_id: String,
    pub queue_entry_id: String,
    pub patient_name: String,
    pub specimen_type: String,
    pub tests: Vec<LabTestView>,
}

#[derive(Template)]
#[template(path = "pages/pharmacy/queue.html")]
pub struct PharmacyQueuePage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub orders: Vec<PharmacyOrderView>,
}

#[derive(Template)]
#[template(path = "pages/pharmacy/dispense.html")]
pub struct PharmacyDispensePage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub order_id: String,
    pub queue_entry_id: String,
    pub patient_name: String,
    pub items: Vec<PrescriptionItemView>,
}

#[derive(Template)]
#[template(path = "pages/pharmacy/dispense_slip_print.html")]
pub struct PharmacyDispenseSlipPrintPage {
    pub order_id: String,
    pub patient_name: String,
    pub items: Vec<PrescriptionItemView>,
}

#[derive(Template)]
#[template(path = "pages/sync/status.html")]
pub struct SyncStatusPage {
    pub active_nav: String,
    pub display_name: String,
    pub initials: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    pub pending_ops_count: u64,
    pub conflict_queue_depth: u32,
    pub cursor_lag: u64,
    pub last_push_at: String,
    pub last_pull_at: String,
    pub conflicts: Vec<SyncConflictView>,
}

// ===== Simple board partials =====

#[derive(Template)]
#[template(path = "partials/simple_queue_row.html")]
pub struct SimpleQueueRowPartial {
    pub p: SimpleQueuePatientView,
}

// ===== View models =====

#[derive(Clone)]
pub struct PatientView {
    pub id: String,
    pub given_name: String,
    pub family_name: String,
    pub sex: String,
    pub date_of_birth: String,
    pub national_id: String,
    pub phone: String,
}

#[derive(Clone)]
pub struct PatientTimelineItemView {
    pub entry_type: String,
    pub title: String,
    pub detail: String,
    pub occurred_at: String,
}

#[derive(Clone)]
pub struct QueueEntryView {
    pub id: String,
    pub position: u32,
    pub patient_name: String,
    pub service_type: String,
    pub department: String,
    pub status: String,
    pub wait_time: String,
    pub assigned_to_name: String,
}

#[derive(Clone)]
pub struct LabOrderView {
    pub order_id: String,
    pub queue_entry_id: String,
    pub patient_name: String,
    pub test_count: usize,
    pub priority: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Clone)]
pub struct LabTestView {
    pub test_code: String,
    pub test_name: String,
    pub result: String,
    pub result_value: String,
    pub unit: String,
    pub reference_range: String,
    pub is_abnormal: bool,
}

#[derive(Clone)]
pub struct PharmacyOrderView {
    pub order_id: String,
    pub queue_entry_id: String,
    pub patient_name: String,
    pub item_count: usize,
    pub priority: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Clone)]
pub struct SimpleQueuePatientView {
    pub queue_entry_id: String,
    pub encounter_id: String,
    pub position: u32,
    pub patient_name: String,
    pub journey_status: String,
    pub wait_time: String,
}

#[derive(Clone)]
pub struct PrescriptionItemView {
    pub medication_name: String,
    pub medication_field_name: String,
    pub dosage: String,
    pub frequency: String,
    pub duration: String,
    pub quantity: u32,
    pub dispensed_quantity: String,
}

#[derive(Clone)]
pub struct SyncConflictView {
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub conflict_type: String,
    pub created_at: String,
}
