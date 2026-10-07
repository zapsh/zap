// SPDX-License-Identifier: AGPL-3.0-only

/**
 * {
        id: 1,
        name: '技术',
        sort: 1,
        articleCount: 10,
        createTime: '2024-01-20 12:00:00',
      }

 */

export interface Category {
  id: number;
  name: string;
  sort: number; // 排序
  articleCount: number; // 文章数量
  createTime: string; // 创建时间
}