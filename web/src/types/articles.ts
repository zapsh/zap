// SPDX-License-Identifier: AGPL-3.0-only
// 生成articles 类型/**
/**
 * {
        id: 1,
        title: '示例文章',
        category: '技术',
        tags: ['Vue', 'TypeScript'],
        status: 1,
        createTime: '2024-01-20 12:00:00',
      }
 */


     /**
      * 文章类型定义
      */
     export interface Article {
       id: number;
       title: string;
       category: string;
       tags: string[];
       status: number;
       createTime: string;
     }
     