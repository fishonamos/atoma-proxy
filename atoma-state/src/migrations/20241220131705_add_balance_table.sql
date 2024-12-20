CREATE TABLE balance (
  user_id INTEGER PRIMARY KEY,
  usdc_balance BIGINT NOT NULL CHECK (usdc_balance > 0),
  usdc_last_timestamp BIGINT NOT NULL,
  FOREIGN KEY (user_id) REFERENCES users(id)
);

CREATE UNIQUE INDEX idx_unique_sui_address ON users(sui_address) WHERE sui_address IS NOT NULL;
