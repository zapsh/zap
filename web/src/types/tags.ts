// SPDX-License-Identifier: AGPL-3.0-only
/**
 * {
        id: 1,
        name: 'Vue3',
        type: 'success',
        color: '',
        articleCount: 5,
        createTime: '2024-01-20 12:00:00',
      },
 */

export interface Tag {
  id: number; // 标签ID
  name: string; // 标签名称
  type: 'success' | 'info' | 'warning' | 'danger'; // 标签类型
  color?: string; // 标签颜色
  articleCount: number; // 关联文章数量
  createTime: string; // 创建时间
}