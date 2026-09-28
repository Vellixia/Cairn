-- Server receipt time is authoritative for whether an installation report is stale.
-- `observed_at` remains the reporter's claim about when its check ran.
ALTER TABLE integration_health
  ADD COLUMN reported_at TIMESTAMPTZ NOT NULL DEFAULT now();
