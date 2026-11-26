-- FrankenForecast (fcast) Migration 0004: Immutability triggers on evidence_vault_index
-- and question_gold_dossiers (INV-01, INV-03, §7.3).

CREATE TRIGGER IF NOT EXISTS trg_evidence_vault_index_no_update
BEFORE UPDATE ON evidence_vault_index
BEGIN
    SELECT RAISE(ABORT, 'IMMUTABLE_EVIDENCE_VAULT_INDEX');
END;

CREATE TRIGGER IF NOT EXISTS trg_evidence_vault_index_no_delete
BEFORE DELETE ON evidence_vault_index
BEGIN
    SELECT RAISE(ABORT, 'IMMUTABLE_EVIDENCE_VAULT_INDEX');
END;

CREATE TRIGGER IF NOT EXISTS trg_question_gold_dossiers_no_update
BEFORE UPDATE ON question_gold_dossiers
BEGIN
    SELECT RAISE(ABORT, 'IMMUTABLE_GOLD_DOSSIER');
END;

CREATE TRIGGER IF NOT EXISTS trg_question_gold_dossiers_no_delete
BEFORE DELETE ON question_gold_dossiers
BEGIN
    SELECT RAISE(ABORT, 'IMMUTABLE_GOLD_DOSSIER');
END;
