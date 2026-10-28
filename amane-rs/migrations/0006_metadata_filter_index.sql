-- 类型筛选是界面上的常用条件; 按更新时间倒序的索引已由 0001 建立.
CREATE INDEX idx_metadata_content_type ON metadata (content_type, updated_at DESC);
